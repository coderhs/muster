<script lang="ts" module>
  import { keyText } from "../os";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import type { Id } from "../protocol/types";

  const key = (pid: Id, path: string) => `${pid}\n${path}`;
  /** The document last read in each project, so that coming back to the tab keeps the place. */
  const lastRead = new Map<Id, string>();
  /** Text typed and not saved, by project and path. It outlives a change of tab or project. */
  const drafts = new SvelteMap<string, string>();
  /** Folders closed in the list, by project and folder path. */
  const closed = new SvelteSet<string>();
</script>

<script lang="ts">
  import { tick, untrack } from "svelte";
  import { useApp } from "../state/context";
  import { clock } from "../state/clock.svelte";
  import { errText } from "../protocol/client";
  import type { DocEntry, DocKind } from "../protocol/types";
  import { inline as parseInline, parse, target, type Block, type Inline } from "./markdown";
  import { TIPS, remarks, type TipSet } from "./docTips";
  import { newDocPath, starter, treeRows } from "./docsTree";
  import { bytes } from "./format";
  import { agoLabel } from "./time";
  import Icon from "./Icon.svelte";

  type Guide = Exclude<DocKind, "doc">;
  /** What the page shows: a file, or the tips for an instruction file the project lacks. */
  type Selection = { path: string } | { guide: Guide };

  const app = useApp();
  const projectId = $derived(app.currentProjectId);
  const ready = $derived(app.conn.kind === "ready");

  let docs = $state<DocEntry[] | null>(null);
  let listError = $state("");
  let more = $state(false);
  let selection = $state<Selection | null>(null);
  let page = $state<{ path: string; content: string; modifiedAt: number; cut: boolean } | null>(null);
  let pageError = $state("");
  let tipsOpen = $state(false);
  let editing = $state(false);
  let preview = $state(false);
  let saving = $state(false);
  /** A name being typed for a new document, in the folder it will go to. */
  let naming = $state<{ folder: string; value: string; error: string } | null>(null);
  let pageEl = $state<HTMLElement>();
  let editorEl = $state<HTMLTextAreaElement>();
  /** In a narrow window the list and the page take turns. */
  let narrow = $state(false);
  let reading = $state(false);
  let loads = 0;

  const instructions = $derived((docs ?? []).filter((d) => d.kind !== "doc"));
  const missing = $derived((["claude", "agents"] as const).filter((k) => docs && !docs.some((d) => d.kind === k)));
  const closedHere = $derived.by(() => {
    const prefix = `${projectId}\n`;
    return new Set([...closed].filter((k) => k.startsWith(prefix)).map((k) => k.slice(prefix.length)));
  });
  const tree = $derived(treeRows((docs ?? []).filter((d) => d.kind === "doc"), closedHere));
  const path = $derived(selection && "path" in selection ? selection.path : null);
  const entry = $derived(path ? ((docs ?? []).find((d) => d.path === path) ?? null) : null);
  const shown = $derived(page && page.path === path ? page : null);
  const draft = $derived(projectId && path ? drafts.get(key(projectId, path)) : undefined);
  const text = $derived(draft ?? shown?.content ?? "");
  const blocks = $derived(shown && (!editing || preview) ? parse(text) : []);
  const notes = $derived(shown && entry ? remarks(entry.kind, text, docs ?? []) : []);
  const guide = $derived<TipSet | null>(
    selection && "guide" in selection ? TIPS[selection.guide] : entry && entry.kind !== "doc" && tipsOpen ? TIPS[entry.kind] : null,
  );

  $effect(() => {
    const mq = window.matchMedia("(max-width: 720px)");
    const set = () => (narrow = mq.matches);
    set();
    mq.addEventListener("change", set);
    return () => mq.removeEventListener("change", set);
  });

  // The list is read when the tab opens, when the project changes and when the daemon
  // comes back; after that only on Refresh. Nothing is watched.
  $effect(() => {
    const pid = projectId;
    if (!pid || !ready) return;
    untrack(() => void loadList(pid, true));
  });

  async function loadList(pid: Id, fresh: boolean, want?: string) {
    const n = ++loads;
    if (fresh) {
      docs = null;
      selection = null;
      page = null;
      pageError = "";
      reading = false;
      naming = null;
    }
    listError = "";
    try {
      const r = await app.client.request("docs.list", { projectId: pid });
      if (n !== loads) return;
      docs = r.docs;
      more = r.truncated ?? false;
      const keep = want ?? (selection && "path" in selection ? selection.path : lastRead.get(pid));
      const again = r.docs.find((d) => d.path === keep);
      if (again) await open(again.path);
      else if (!selection || "path" in selection) {
        if (r.docs.length) await open(r.docs[0].path);
        else selection = { guide: "claude" };
      }
    } catch (e) {
      if (n !== loads) return;
      docs = [];
      listError = errText(e);
    }
  }

  async function open(p: string, anchor = "") {
    const pid = projectId;
    if (!pid) return;
    const n = ++loads;
    selection = { path: p };
    lastRead.set(pid, p);
    pageError = "";
    // Unsaved text is still there: go on editing it.
    editing = drafts.has(key(pid, p));
    preview = false;
    try {
      const r = await app.client.request("docs.read", { projectId: pid, path: p });
      if (n !== loads) return;
      page = { path: p, content: r.content, modifiedAt: r.modifiedAt, cut: r.truncated ?? false };
      requestAnimationFrame(() => (anchor ? scrollTo(anchor) : pageEl?.scrollTo({ top: 0 })));
    } catch (e) {
      if (n !== loads) return;
      page = null;
      pageError = errText(e);
    }
  }

  function choose(s: Selection) {
    reading = true;
    tipsOpen = false;
    if ("path" in s) void open(s.path);
    else {
      loads++;
      selection = s;
      page = null;
      pageError = "";
      editing = false;
      pageEl?.scrollTo({ top: 0 });
    }
  }

  function scrollTo(anchor: string) {
    pageEl?.querySelector(`#${CSS.escape(`doc-${anchor.toLowerCase()}`)}`)?.scrollIntoView({ block: "start" });
  }

  function follow(e: MouseEvent, href: string) {
    e.preventDefault();
    const t = target(href, path ?? "");
    if (t.kind === "web") {
      void app.platform.openUrl(t.url).catch((err) => app.toast(`Could not open the link: ${errText(err)}`));
    } else if (t.kind === "anchor") scrollTo(t.anchor);
    else if (t.kind === "doc") {
      // Links are often written in another case than the file's name.
      const hit = docs?.find((d) => d.path === t.path) ?? docs?.find((d) => d.path.toLowerCase() === t.path.toLowerCase());
      if (hit) void open(hit.path, t.anchor);
      else app.toast(`${t.path} is not one of this project's documents`, "info");
    }
  }

  async function startEdit() {
    editing = true;
    preview = false;
    await tick();
    editorEl?.focus();
  }

  function typed(value: string) {
    if (!projectId || !shown) return;
    const k = key(projectId, shown.path);
    if (value === shown.content) drafts.delete(k);
    else drafts.set(k, value);
  }

  function discard() {
    if (projectId && path) drafts.delete(key(projectId, path));
    editing = false;
    preview = false;
  }

  async function save() {
    const pid = projectId;
    if (!pid || !shown || saving) return;
    if (draft === undefined) {
      editing = false;
      return;
    }
    const content = draft;
    saving = true;
    try {
      const r = await app.client.request("docs.write", { projectId: pid, path: shown.path, content, expectedModifiedAt: shown.modifiedAt });
      // Typing went on while it was being saved: that stays a draft.
      if (drafts.get(key(pid, r.doc.path)) === content) drafts.delete(key(pid, r.doc.path));
      docs = (docs ?? []).map((d) => (d.path === r.doc.path ? r.doc : d));
      if (page?.path === r.doc.path) {
        page = { ...page, content, modifiedAt: r.doc.modifiedAt };
        if (!drafts.has(key(pid, r.doc.path))) editing = false;
      }
    } catch (e) {
      app.toast(`Not saved: ${errText(e)}`);
    } finally {
      saving = false;
    }
  }

  async function create(p: string, kind: DocKind) {
    const pid = projectId;
    if (!pid) return false;
    const content = starter(p, kind, app.currentProject?.name ?? "Project", (docs ?? []).some((d) => d.kind === "agents"));
    try {
      await app.client.request("docs.write", { projectId: pid, path: p, content, create: true });
    } catch (e) {
      if (naming) naming.error = errText(e);
      else app.toast(`Not created: ${errText(e)}`);
      return false;
    }
    // Its folders open, so that the new file is in sight.
    const parts = p.split("/");
    for (let i = 1; i < parts.length; i++) closed.delete(key(pid, `${parts.slice(0, i).join("/")}/`));
    reading = true;
    tipsOpen = false;
    await loadList(pid, false, p);
    if (path === p) await startEdit();
    return true;
  }

  async function submitName() {
    if (!naming) return;
    const r = newDocPath(naming.value, naming.folder);
    if ("error" in r) naming.error = r.error;
    else if (await create(r.path, "doc")) naming = null;
  }

  function editorKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      void save();
    } else if (e.key === "Tab" && !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey) {
      // Indents, as a list in Markdown needs; Escape then Tab leaves the editor.
      e.preventDefault();
      const el = e.currentTarget as HTMLTextAreaElement;
      el.setRangeText("  ", el.selectionStart, el.selectionEnd, "end");
      typed(el.value);
    } else if (e.key === "Escape") (e.currentTarget as HTMLTextAreaElement).blur();
  }

  function toggle(folder: string) {
    if (!projectId) return;
    const k = key(projectId, folder);
    if (!closed.delete(k)) closed.add(k);
  }

  /** Where a name starts in a row without a folder's arrow, and how far each level moves it. */
  const FLAT = 8;
  const STEP = 14;
  /** A file's name starts where the names of the folders beside it do, past their arrows. */
  const fileIndent = (depth: number) => (tree.some((r) => r.t === "folder") ? 26 + depth * STEP : FLAT);

  const folderOf = (p: string) => (p.includes("/") ? p.slice(0, p.lastIndexOf("/") + 1) : "");
  const nameOf = (p: string) => p.slice(p.lastIndexOf("/") + 1);
  const unsaved = (d: DocEntry) => !!projectId && drafts.has(key(projectId, d.path));
  const focusNow = (el: HTMLElement) => el.focus();
</script>

{#snippet inl(nodes: Inline[])}{#each nodes as n}{#if n.t === "text"}{n.s}{:else if n.t === "code"}<code>{n.s}</code>{:else if n.t === "strong"}<strong>{@render inl(n.c)}</strong>{:else if n.t === "em"}<em>{@render inl(n.c)}</em>{:else if n.t === "del"}<del>{@render inl(n.c)}</del>{:else if n.t === "link"}<a href={n.href} title={n.href} onclick={(e) => follow(e, n.href)}>{@render inl(n.c)}</a>{:else}<br />{/if}{/each}{/snippet}

{#snippet body(bs: Block[])}
  {#each bs as b}
    {#if b.t === "heading"}
      <svelte:element this={`h${b.level}`} id="doc-{b.id}">{@render inl(b.c)}</svelte:element>
    {:else if b.t === "p"}
      <p>{@render inl(b.c)}</p>
    {:else if b.t === "code"}
      <pre><code>{b.s}</code></pre>
    {:else if b.t === "quote"}
      <blockquote>{@render body(b.c)}</blockquote>
    {:else if b.t === "list"}
      <svelte:element this={b.ordered ? "ol" : "ul"} start={b.ordered && b.start !== 1 ? b.start : undefined}>
        {#each b.items as item}
          <li class:task={item.checked !== undefined}>
            {#if item.checked !== undefined}<input type="checkbox" checked={item.checked} disabled aria-label={item.checked ? "Done" : "Not done"} />{/if}
            {@render body(item.c)}
          </li>
        {/each}
      </svelte:element>
    {:else if b.t === "table"}
      <div class="table">
        <table>
          <thead>
            <tr>{#each b.head as h, i}<th style:text-align={b.align[i] || undefined}>{@render inl(h)}</th>{/each}</tr>
          </thead>
          <tbody>
            {#each b.rows as row}
              <tr>{#each row as cell, i}<td style:text-align={b.align[i] || undefined}>{@render inl(cell)}</td>{/each}</tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <hr />
    {/if}
  {/each}
{/snippet}

{#snippet tips(set: TipSet, absent: Guide | null)}
  <section class="tips" aria-label="Tips for {set.file}">
    <h2>{absent ? `This project has no ${set.file}` : `Tips for ${set.file}`}</h2>
    <p class="what">{@render inl(parseInline(set.what))}</p>
    <dl>
      {#each set.tips as t}
        <dt>{t.head}</dt>
        <dd>{@render inl(parseInline(t.text))}</dd>
      {/each}
    </dl>
    {#if absent}
      <button class="btn primary" disabled={!ready} onclick={() => void create(set.file, absent)}><Icon name="plus" size={12} />Create {set.file}</button>
    {/if}
  </section>
{/snippet}

{#snippet file(d: DocEntry, indent: number, label: string, dir: string)}
  <button
    class="row"
    class:on={path === d.path}
    style:padding-left="{indent}px"
    onclick={() => choose({ path: d.path })}
    title={d.path}
  >
    <span class="name">{#if dir}<span class="dir">{dir}</span>{/if}{label}</span>
    {#if unsaved(d)}<span class="dirty" title="Unsaved changes"></span>{/if}
  </button>
{/snippet}

{#snippet nameField(indent: number)}
  {#if naming}
    <form
      class="naming"
      style:padding-left="{indent - 6}px"
      onsubmit={(e) => {
        e.preventDefault();
        void submitName();
      }}
    >
      <input
        class="field"
        placeholder="File name"
        aria-label="Name of the new document{naming.folder ? ` in ${naming.folder}` : ''}"
        bind:value={naming.value}
        use:focusNow
        oninput={() => naming && (naming.error = "")}
        onkeydown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            naming = null;
          }
        }}
        onblur={() => {
          if (naming && !naming.value.trim()) naming = null;
        }}
      />
      {#if naming.error}<p class="quiet err">{naming.error}</p>{/if}
    </form>
  {/if}
{/snippet}

<div class="docs" class:narrow class:reading>
  <nav class="list" aria-label="Documents">
    {#if docs === null}
      <p class="quiet">Loading…</p>
    {:else}
      <h3>Agent instructions</h3>
      {#each instructions as d (d.path)}
        {@render file(d, FLAT, nameOf(d.path), folderOf(d.path))}
      {/each}
      {#each missing as k (k)}
        <button class="row absent" class:on={!!selection && "guide" in selection && selection.guide === k} style:padding-left="8px" onclick={() => choose({ guide: k })}>
          <span class="name">{TIPS[k].file}</span>
          <span class="hint">not added</span>
        </button>
      {/each}
      <div class="head">
        <h3>Documents</h3>
        <button class="icon-btn" title="New document" disabled={!ready} onclick={() => (naming = { folder: "", value: "", error: "" })}><Icon name="plus" size={13} /></button>
      </div>
      {#if naming && naming.folder === ""}{@render nameField(fileIndent(0))}{/if}
      {#each tree as r (r.t === "folder" ? r.path : r.doc.path)}
        {#if r.t === "folder"}
          <div class="folder" style:padding-left="{r.depth * STEP}px">
            <button class="fold" aria-expanded={r.open} onclick={() => toggle(r.path)} title={r.path}>
              <span class="chev" class:open={r.open}><Icon name="chevronRight" size={11} /></span>
              <span class="name">{r.name}</span>
              {#if !r.open}<span class="hint">{r.count}</span>{/if}
            </button>
            <button
              class="icon-btn add"
              title="New document in {r.path}"
              disabled={!ready}
              onclick={() => {
                if (projectId) closed.delete(key(projectId, r.path));
                naming = { folder: r.path, value: "", error: "" };
              }}><Icon name="plus" size={12} /></button
            >
          </div>
          {#if naming && naming.folder === r.path}{@render nameField(fileIndent(r.depth + 1))}{/if}
        {:else}
          {@render file(r.doc, fileIndent(r.depth), r.name, "")}
        {/if}
      {/each}
      {#if !tree.length && !naming}<p class="quiet">No documents yet.</p>{/if}
      {#if more}<p class="quiet">Only the first {docs.length} files are listed.</p>{/if}
      {#if listError}<p class="quiet err">{listError}</p>{/if}
    {/if}
  </nav>

  <div class="page" class:editing={editing && !preview && !!shown} bind:this={pageEl}>
    {#if selection}
      <header>
        {#if narrow}<button class="icon-btn" title="All documents" onclick={() => (reading = false)}><Icon name="chevronLeft" /></button>{/if}
        <span class="file" title={path ?? undefined}>{path ?? ("guide" in selection ? TIPS[selection.guide].file : "")}</span>
        {#if entry}
          <span class="meta">
            {#if draft !== undefined}<span class="unsaved">Unsaved changes</span>{:else}{bytes(entry.size)} · changed {agoLabel(entry.modifiedAt, clock.now)}{/if}
          </span>
        {/if}
        <span class="spacer"></span>
        {#if entry && entry.kind !== "doc"}
          <button class="btn small" aria-pressed={tipsOpen} class:pressed={tipsOpen} onclick={() => (tipsOpen = !tipsOpen)}>Tips</button>
        {/if}
        {#if shown && editing}
          <button class="btn small" aria-pressed={preview} class:pressed={preview} onclick={() => (preview = !preview)}>Preview</button>
          <button class="btn small" disabled={saving} onclick={discard}>{draft === undefined ? "Close" : "Discard"}</button>
          <button class="btn small primary" disabled={saving || !ready || draft === undefined} onclick={() => void save()} title={keyText("Save (⌘S)")}>{saving ? "Saving…" : "Save"}</button>
        {:else}
          {#if shown}
            <button class="btn small" disabled={shown.cut || !ready} onclick={() => void startEdit()} title={shown.cut ? "This file is too long to edit here" : "Edit this file"}>
              <Icon name="pencil" size={12} />Edit
            </button>
          {/if}
          <button class="btn small" disabled={!ready} onclick={() => projectId && void loadList(projectId, false)} title="Read the files again">Refresh</button>
        {/if}
      </header>
    {/if}
    {#if notes.length || guide}
      <div class="above">
        {#if notes.length}
          <ul class="notes">
            {#each notes as note}<li>{@render inl(parseInline(note))}</li>{/each}
          </ul>
        {/if}
        {#if guide}{@render tips(guide, selection && "guide" in selection ? selection.guide : null)}{/if}
      </div>
    {/if}
    {#if pageError}
      <p class="quiet err pad">{pageError}</p>
    {:else if shown && editing && !preview}
      <textarea
        class="editor"
        bind:this={editorEl}
        value={text}
        spellcheck="false"
        aria-label="Text of {shown.path}"
        oninput={(e) => typed(e.currentTarget.value)}
        onkeydown={editorKey}
      ></textarea>
    {:else if shown}
      <article>
        {#if shown.cut}<p class="quiet">This file is long; only its beginning is shown.</p>{/if}
        {@render body(blocks)}
        {#if !blocks.length}<p class="quiet">This file is empty.</p>{/if}
      </article>
    {/if}
  </div>
</div>

<style>
  .docs {
    flex: 1;
    min-height: 0;
    display: flex;
    border-top: 1px solid var(--border);
  }
  .list {
    width: 240px;
    flex: none;
    overflow-y: auto;
    padding: 6px 8px 16px;
    border-right: 1px solid var(--border);
  }
  .list h3 {
    margin: 12px 8px 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
  }
  .head .icon-btn {
    width: 22px;
    height: 22px;
  }
  .row,
  .fold {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 26px;
    padding: 4px 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
    font-size: 12.5px;
    min-width: 0;
  }
  .row {
    width: 100%;
  }
  .row:hover,
  .folder:hover {
    background: var(--bg-hover);
  }
  .row.on {
    background: var(--bg-active);
  }
  .folder {
    display: flex;
    align-items: center;
    border-radius: 6px;
    padding-right: 2px;
  }
  .fold {
    flex: 1;
    gap: 4px;
    padding-left: 8px;
    color: var(--text-2);
  }
  .chev {
    display: inline-grid;
    place-items: center;
    width: 14px;
    flex: none;
    color: var(--text-3);
    transition: transform 0.12s ease-out;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .folder .add {
    width: 22px;
    height: 22px;
    flex: none;
    opacity: 0;
  }
  .folder:hover .add,
  .folder .add:focus-visible {
    opacity: 1;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dir,
  .hint {
    color: var(--text-3);
  }
  .hint {
    flex: none;
    font-size: 11px;
  }
  .absent .name {
    color: var(--text-2);
  }
  .dirty {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--s-needs);
  }
  .naming {
    padding: 2px 4px 4px 0;
  }
  .naming .field {
    width: 100%;
    height: 24px;
    font-size: 12.5px;
  }
  .naming .quiet {
    margin: 4px 2px 0;
  }
  .quiet {
    margin: 8px;
    color: var(--text-3);
    font-size: 12px;
  }
  .err {
    color: var(--danger);
    word-break: break-word;
  }
  .pad {
    margin: 18px 24px;
  }
  .page {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .page.editing {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  header {
    position: sticky;
    top: 0;
    z-index: 1;
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 14px 0 24px;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }
  .file {
    font: 12px var(--mono);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    flex: none;
    color: var(--text-3);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .unsaved {
    color: var(--s-needs);
  }
  .spacer {
    flex: 1;
  }
  .btn.pressed {
    background: var(--bg-active);
  }
  .above {
    flex: none;
    max-width: 780px;
    padding: 18px 24px 0;
    font-size: 13.5px;
    line-height: 1.6;
  }
  .editing .above {
    max-height: 40%;
    overflow-y: auto;
  }
  .editor {
    flex: 1;
    min-height: 0;
    width: 100%;
    resize: none;
    border: 0;
    outline: 0;
    padding: 16px 24px 32px;
    background: var(--bg);
    color: var(--text);
    font: 12.5px/1.6 var(--mono);
    tab-size: 2;
  }
  article {
    max-width: 780px;
    padding: 18px 24px 48px;
    font-size: 13.5px;
    line-height: 1.6;
    overflow-wrap: break-word;
    user-select: text;
    -webkit-user-select: text;
  }
  article .quiet {
    margin: 0 0 12px;
  }
  article :is(h1, h2, h3, h4, h5, h6) {
    margin: 1.5em 0 0.5em;
    line-height: 1.3;
    font-weight: 600;
    scroll-margin-top: 44px;
  }
  article > :first-child {
    margin-top: 0;
  }
  article h1 {
    font-size: 21px;
  }
  article h2 {
    font-size: 17px;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border);
  }
  article h3 {
    font-size: 14.5px;
  }
  article :is(h4, h5, h6) {
    font-size: 13.5px;
    color: var(--text-2);
  }
  article p {
    margin: 0 0 0.85em;
  }
  :is(article, .above) a {
    color: var(--accent);
    text-decoration: none;
    cursor: pointer;
  }
  :is(article, .above) a:hover {
    text-decoration: underline;
  }
  /* Inline code only. A block's code is excluded here instead of being overridden
     below: scoping changes the two rules' weights, and the override lost. */
  :is(article, .above) :not(pre) > code {
    font: 0.9em var(--mono);
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--bg-hover);
  }
  article pre {
    margin: 0 0 1em;
    padding: 10px 12px;
    border-radius: var(--radius);
    background: var(--bg-input);
    border: 1px solid var(--border);
    overflow-x: auto;
    line-height: 1.45;
  }
  article pre code {
    font-family: var(--mono);
    padding: 0;
    background: none;
    font-size: 12px;
    white-space: pre;
  }
  article blockquote {
    margin: 0 0 1em;
    padding: 2px 0 2px 12px;
    border-left: 3px solid var(--border-strong);
    color: var(--text-2);
  }
  article blockquote > :global(:last-child),
  article li > :global(:last-child) {
    margin-bottom: 0;
  }
  article :is(ul, ol) {
    margin: 0 0 0.85em;
    padding-left: 22px;
  }
  article li {
    margin: 2px 0;
  }
  article li > p {
    margin-bottom: 0.3em;
  }
  article li.task {
    list-style: none;
    position: relative;
  }
  article li.task > input {
    position: absolute;
    left: -20px;
    top: 4px;
    margin: 0;
  }
  article hr {
    border: 0;
    border-top: 1px solid var(--border-strong);
    margin: 1.4em 0;
  }
  .table {
    margin: 0 0 1em;
    overflow-x: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th,
  td {
    padding: 5px 10px;
    border: 1px solid var(--border);
    text-align: left;
    vertical-align: top;
  }
  th {
    background: var(--bg-hover);
    font-weight: 600;
  }
  .notes {
    list-style: none;
    margin: 0 0 16px;
    padding: 8px 12px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--s-needs) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--s-needs) 30%, transparent);
    font-size: 12.5px;
  }
  .tips {
    margin: 0 0 4px;
    padding: 12px 14px;
    border-radius: var(--radius);
    background: var(--bg-column);
    border: 1px solid var(--border);
    font-size: 12.5px;
  }
  .tips h2 {
    margin: 0 0 6px;
    font-size: 13.5px;
    font-weight: 600;
  }
  .tips .what {
    color: var(--text-2);
    margin: 0 0 10px;
  }
  .tips dl {
    margin: 0;
    display: grid;
    grid-template-columns: minmax(120px, 190px) 1fr;
    gap: 6px 14px;
  }
  .tips dt {
    font-weight: 600;
  }
  .tips dd {
    margin: 0;
    color: var(--text-2);
  }
  .tips .btn {
    margin-top: 12px;
  }
  .narrow .list {
    width: auto;
    flex: 1;
    border-right: 0;
  }
  .narrow .folder .add {
    opacity: 1;
  }
  .narrow .page,
  .narrow.reading .list {
    display: none;
  }
  .narrow.reading .page {
    display: block;
  }
  .narrow.reading .page.editing {
    display: flex;
  }
  .narrow header {
    padding-left: 8px;
  }
  .narrow .meta {
    display: none;
  }
  .narrow article,
  .narrow .editor {
    padding: 14px 16px 40px;
  }
  .narrow .above {
    padding: 14px 16px 0;
  }
  .narrow .tips dl {
    grid-template-columns: 1fr;
    gap: 2px;
  }
  .narrow .tips dd {
    margin-bottom: 8px;
  }
</style>
