<script lang="ts">
  import { tick } from "svelte";
  import type { Card, CardChanges, ChangedFile, ChangeKind, ChangeScope } from "../protocol/types";
  import { errText } from "../protocol/client";
  import { useApp } from "../state/context";
  import { formatReview, lineRef, MAX_MESSAGE, parsePatch, refKey, type DiffLine, type Patch } from "../changes/patch";
  import { review } from "../changes/review.svelte";
  import Icon from "./Icon.svelte";
  import Select from "./Select.svelte";

  let { card, active }: { card: Card; active: boolean } = $props();
  const app = useApp();
  // svelte-ignore state_referenced_locally
  const cardId = card.id;
  const rv = review(cardId);

  let changes = $state.raw<CardChanges | null>(null);
  // What to compare, remembered per card: "" is the automatic base.
  const VIEW_KEY = `muster.changes.${cardId}`;
  const saved = (() => {
    try {
      const v = JSON.parse(localStorage.getItem(VIEW_KEY) ?? "{}");
      return {
        base: typeof v.base === "string" ? v.base : "",
        scope: (["all", "committed", "uncommitted"].includes(v.scope) ? v.scope : "all") as ChangeScope,
      };
    } catch {
      return { base: "", scope: "all" as ChangeScope };
    }
  })();
  let base = $state(saved.base);
  let scope = $state<ChangeScope>(saved.scope);
  function setView(b: string, s: ChangeScope) {
    base = b;
    scope = s;
    editing = null;
    try {
      if (!b && s === "all") localStorage.removeItem(VIEW_KEY);
      else localStorage.setItem(VIEW_KEY, JSON.stringify({ base: b, scope: s }));
    } catch {
      /* ignore */
    }
    void refresh();
  }
  const view = () => ({ id: cardId, ...(base ? { base } : {}), scope });
  const SCOPES: Array<{ v: ChangeScope; label: string; title: string }> = [
    { v: "all", label: "All", title: "Commits and uncommitted changes together" },
    { v: "committed", label: "Committed", title: "Only the branch's commits" },
    { v: "uncommitted", label: "Uncommitted", title: "Only what is not committed yet, new files included" },
  ];
  const baseOptions = $derived([
    { value: "", label: changes ? `Automatic (${changes.defaultBase === "HEAD" ? "uncommitted only" : changes.defaultBase})` : "Automatic" },
    ...(changes?.branches ?? (base ? [base] : [])).map((b) => ({ value: b, label: b })),
  ]);

  let error = $state<string | null>(null);
  let loading = $state(false);
  let selected = $state<string | null>(null);
  let patch = $state.raw<Patch | null>(null);
  let patchTruncated = $state(false);
  let patchError = $state<string | null>(null);
  let patchLoading = $state(false);
  let gen = 0;

  let editing = $state<string | null>(null);
  let editText = $state("");
  let editor = $state<HTMLTextAreaElement>();
  let sending = $state(false);

  const KIND: Record<ChangeKind, { letter: string; label: string }> = {
    added: { letter: "A", label: "Added" },
    modified: { letter: "M", label: "Modified" },
    deleted: { letter: "D", label: "Deleted" },
    renamed: { letter: "R", label: "Renamed" },
    untracked: { letter: "U", label: "New, not yet added to git" },
  };

  const totals = $derived(
    (changes?.files ?? []).reduce((t, f) => ({ add: t.add + f.additions, del: t.del + f.deletions }), { add: 0, del: 0 }),
  );
  const file = $derived(changes?.files.find((f) => f.path === selected) ?? null);
  const commentsHere = (path: string) => rv.comments.filter((c) => c.path === path).length;

  async function refresh() {
    const g = ++gen;
    loading = true;
    try {
      const r = await app.client.request("card.changes", view());
      if (g !== gen) return;
      changes = r;
      error = null;
      if (!r.files.some((f) => f.path === selected)) selected = r.files[0]?.path ?? null;
      await loadPatch(g);
    } catch (e) {
      if (g !== gen) return;
      changes = null;
      error = errText(e);
    } finally {
      if (g === gen) loading = false;
    }
  }

  async function loadPatch(g = gen) {
    const path = selected;
    if (!path) {
      patch = null;
      return;
    }
    patchLoading = true;
    try {
      const r = await app.client.request("card.diff", { ...view(), path });
      if (g !== gen || path !== selected) return;
      patch = parsePatch(r.patch);
      patchTruncated = !!r.truncated;
      patchError = null;
    } catch (e) {
      if (g !== gen || path !== selected) return;
      patch = null;
      patchError = errText(e);
    } finally {
      if (path === selected) patchLoading = false;
    }
  }

  function select(f: ChangedFile) {
    if (f.path === selected) return;
    selected = f.path;
    editing = null;
    patch = null;
    void loadPatch();
  }

  // Load when the tab is shown, and again when the agent finishes a turn.
  let wasActive = false;
  $effect(() => {
    if (active && !wasActive) void refresh();
    wasActive = active;
  });
  // svelte-ignore state_referenced_locally
  let lastStatus = card.status;
  $effect(() => {
    const s = card.status;
    if (s !== lastStatus && (s === "done" || s === "idle") && active) void refresh();
    lastStatus = s;
  });

  // ---- comments ----
  function openEditor(l: DiffLine) {
    if (!selected) return;
    const ref = lineRef(selected, l);
    editing = refKey(ref);
    editText = rv.find(ref)?.text ?? "";
    void tick().then(() => editor?.focus());
  }

  function saveEditor(l: DiffLine) {
    if (!selected) return;
    rv.put({ ...lineRef(selected, l), quote: l.text, text: editText });
    editing = null;
  }

  function editorKey(e: KeyboardEvent, l: DiffLine) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      saveEditor(l);
    } else if (e.key === "Escape") {
      // Stays in the card: the modal closes on an Escape nobody handled.
      e.preventDefault();
      editing = null;
    }
  }

  const message = $derived(rv.comments.length || rv.note.trim() ? formatReview(rv.comments, rv.note) : "");
  const tooLong = $derived(message.length > MAX_MESSAGE);
  const sendBlocked = $derived(
    card.agentId === "shell"
      ? "A shell card has no agent to read the review"
      : tooLong
        ? `The review is ${message.length} characters; messages are limited to ${MAX_MESSAGE}`
        : null,
  );

  async function send() {
    if (!message || sendBlocked || sending) return;
    sending = true;
    try {
      await app.call("message.send", { cardId, text: message });
      rv.clear();
      editing = null;
      app.toast(
        !card.hasSession
          ? "Review queued; the agent gets it when its session runs again."
          : card.status === "running" || card.status === "needs_you"
            ? "Review queued; the agent gets it when its current turn ends."
            : "Review sent to the agent.",
        "info",
      );
    } catch {
      /* toast shown */
    } finally {
      sending = false;
    }
  }

  function discard() {
    rv.clear();
    editing = null;
  }

  const split = (path: string) => {
    const i = path.lastIndexOf("/");
    return i < 0 ? { dir: "", name: path } : { dir: path.slice(0, i + 1), name: path.slice(i + 1) };
  };
  const plural = (n: number, one: string, many = one + "s") => `${n} ${n === 1 ? one : many}`;
</script>

<div class="changes">
  <div class="bar">
    <span class="dim small">Compare to</span>
    <Select label="Compare to" compact value={base} options={baseOptions} onchange={(v) => setView(v, scope)} />
    <div class="seg" role="radiogroup" aria-label="Which changes">
      {#each SCOPES as s (s.v)}
        <button role="radio" aria-checked={scope === s.v} class:on={scope === s.v} title={s.title} onclick={() => setView(base, s.v)}>
          {s.label}
        </button>
      {/each}
    </div>
    {#if changes}
      <span class="what">
        <b>{changes.branch ?? "detached HEAD"}</b>
        <span class="dim">
          {#if changes.base !== "HEAD"}· {plural(changes.commits, "commit")}{/if}{changes.uncommitted ? " · uncommitted changes" : ""}
        </span>
      </span>
      {#if changes.files.length}
        <span class="dim">· {plural(changes.files.length, "file")}{changes.truncated ? "+" : ""}</span>
        <span class="add">+{totals.add}</span><span class="del">−{totals.del}</span>
      {/if}
    {:else if loading}
      <span class="dim">Reading changes…</span>
    {/if}
    <span class="grow"></span>
    <button class="icon-btn" title="Refresh" disabled={loading} onclick={refresh}><Icon name="restore" size={13} /></button>
  </div>

  {#if error}
    <div class="empty">
      <Icon name="warning" size={16} />
      <p>{error.includes("not in a git repository") ? "This card's folder is not in a git repository, so there are no changes to show." : error}</p>
      {#if base}<button class="btn small" onclick={() => setView("", scope)}>Compare to the automatic base</button>{/if}
    </div>
  {:else if changes && !changes.files.length}
    <div class="empty">
      <Icon name="check" size={16} />
      <p>
        {changes.scope === "uncommitted" || (changes.base === "HEAD" && changes.scope === "all")
          ? "No uncommitted changes."
          : changes.scope === "committed"
            ? changes.base === "HEAD"
              ? "This card works on the base branch itself, so there are no commits to compare. Pick a branch to compare to."
              : `No commits since ${changes.base}.`
            : `No changes against ${changes.base} yet.`}
      </p>
    </div>
  {:else if changes}
    <div class="body">
      <div class="files" role="listbox" aria-label="Changed files">
        {#each changes.files as f (f.path)}
          {@const p = split(f.path)}
          {@const n = commentsHere(f.path)}
          <button
            class="file"
            class:on={f.path === selected}
            role="option"
            aria-selected={f.path === selected}
            title={`${KIND[f.kind].label}: ${f.oldPath ? `${f.oldPath} → ` : ""}${f.path}`}
            onclick={() => select(f)}
          >
            <span class="k k-{f.kind}">{KIND[f.kind].letter}</span>
            <span class="path"><span class="name">{p.name}</span>{#if p.dir}<span class="dir">{p.dir}</span>{/if}</span>
            {#if n}<span class="cnt" title={plural(n, "comment")}>{n}</span>{/if}
            {#if f.binary}<span class="dim small">bin</span>{:else}<span class="nums"><span class="add">+{f.additions}</span><span class="del">−{f.deletions}</span></span>{/if}
          </button>
        {/each}
        {#if changes.truncated}<p class="dim small more">Only the first {changes.files.length} files are listed.</p>{/if}
      </div>

      <div class="diff">
        {#if file}
          <div class="fhead">
            <span class="k k-{file.kind}">{KIND[file.kind].letter}</span>
            <span class="mono">{file.oldPath ? `${file.oldPath} → ` : ""}{file.path}</span>
            <span class="grow"></span>
            <span class="dim small">Click a line to comment</span>
          </div>
        {/if}
        {#if patchError}
          <div class="note err"><Icon name="warning" size={12} />{patchError}</div>
        {:else if patch}
          {#each patch.notes as n}<div class="note">{n}</div>{/each}
          {#if patch.binary}
            <div class="note">Binary file; no text to show.</div>
          {:else if !patch.hunks.length && !patch.notes.length}
            <div class="note">No line changes{file?.kind === "untracked" ? " (empty file)" : ""}.</div>
          {/if}
          <div class="code">
            {#each patch.hunks as h, hi (hi)}
              {@const at = h.lines.find((l) => l.new != null)?.new ?? h.lines[0]?.old}
              <div class="hunk">{at != null ? `Line ${at}` : ""}{h.header ? ` · ${h.header}` : ""}</div>
              {#each h.lines as l, li (li)}
                {@const ref = selected ? lineRef(selected, l) : null}
                {@const key = ref ? refKey(ref) : ""}
                {@const c = ref ? rv.find(ref) : undefined}
                <button class="ln {l.kind}" class:has={!!c} onclick={() => openEditor(l)} title="Comment on this line">
                  <span class="no">{l.old ?? ""}</span><span class="no">{l.new ?? ""}</span><span class="sign"
                    >{l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}</span
                  ><span class="txt">{l.text}</span>
                </button>
                {#if editing === key}
                  <div class="cbox">
                    <textarea
                      bind:this={editor}
                      bind:value={editText}
                      rows="3"
                      placeholder="What should the agent change here?"
                      onkeydown={(e) => editorKey(e, l)}
                    ></textarea>
                    <div class="cactions">
                      <span class="dim small"><kbd>⌘</kbd><kbd>↩</kbd> save · <kbd>esc</kbd> cancel</span>
                      <span class="grow"></span>
                      {#if c}<button class="btn small" onclick={() => { editText = ""; saveEditor(l); }}>Delete</button>{/if}
                      <button class="btn small" onclick={() => (editing = null)}>Cancel</button>
                      <button class="btn small primary" disabled={!editText.trim() && !c} onclick={() => saveEditor(l)}>Save</button>
                    </div>
                  </div>
                {:else if c}
                  <button class="comment" onclick={() => openEditor(l)} title="Edit comment">
                    <Icon name="pencil" size={11} /><span>{c.text}</span>
                  </button>
                {/if}
              {/each}
            {/each}
          </div>
          {#if patchTruncated}<div class="note">The diff is too long to show in full.</div>{/if}
        {:else if patchLoading}
          <div class="note dim">Loading…</div>
        {/if}
      </div>
    </div>
  {/if}

  {#if changes?.files.length || rv.comments.length || rv.note}
    <div class="review">
      <textarea
        class="overall"
        rows="1"
        placeholder="A note for the whole review (optional)"
        value={rv.note}
        oninput={(e) => (rv.note = e.currentTarget.value)}
      ></textarea>
      <span class="dim small">{plural(rv.comments.length, "comment")}</span>
      {#if rv.comments.length || rv.note}<button class="btn small" onclick={discard}>Discard</button>{/if}
      <button
        class="btn small primary"
        disabled={!message || !!sendBlocked || sending}
        title={sendBlocked ?? "Send the comments to the agent as one message"}
        onclick={send}
      >
        <Icon name="mail" size={11} />Send to agent
      </button>
    </div>
  {/if}
</div>

<style>
  .changes {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-term);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    min-height: 34px;
  }
  .seg {
    display: flex;
    flex: none;
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    border: 0;
    border-radius: 0;
    background: transparent;
    padding: 2px 8px;
    font-size: 11.5px;
    color: var(--text-2);
  }
  .seg button + button {
    border-left: 1px solid var(--border-strong);
  }
  .seg button:hover {
    background: var(--bg-hover);
  }
  .seg button.on {
    background: var(--bg-active);
    color: var(--text);
  }
  .bar .what {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar .icon-btn {
    width: 24px;
    height: 24px;
  }
  .grow {
    flex: 1;
  }
  .dim {
    color: var(--text-3);
  }
  .small {
    font-size: 11px;
  }
  .mono {
    font-family: var(--mono);
  }
  .add {
    color: var(--s-done);
    font-variant-numeric: tabular-nums;
  }
  .del {
    color: var(--danger);
    font-variant-numeric: tabular-nums;
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text-2);
    font-size: 13px;
    padding: 24px;
    text-align: center;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .files {
    width: 240px;
    flex: none;
    overflow-y: auto;
    padding: 6px;
    border-right: 1px solid var(--border);
  }
  .file {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-height: 26px;
    padding: 3px 6px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
    font-size: 12px;
    min-width: 0;
  }
  .file:hover {
    background: var(--bg-hover);
  }
  .file.on {
    background: var(--bg-active);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    direction: ltr;
  }
  .path .dir {
    color: var(--text-3);
    margin-left: 6px;
    font-size: 11px;
  }
  .nums {
    display: flex;
    gap: 4px;
    font-size: 11px;
    flex: none;
  }
  .cnt {
    flex: none;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-text);
    font-size: 10px;
    line-height: 16px;
    text-align: center;
  }
  .more {
    padding: 6px;
  }
  .k {
    flex: none;
    width: 16px;
    height: 16px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 700;
    line-height: 16px;
    text-align: center;
    font-family: var(--mono);
  }
  .k-added,
  .k-untracked {
    color: var(--s-done);
    background: color-mix(in srgb, var(--s-done) 16%, transparent);
  }
  .k-modified {
    color: var(--s-needs);
    background: color-mix(in srgb, var(--s-needs) 16%, transparent);
  }
  .k-deleted {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 16%, transparent);
  }
  .k-renamed {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .diff {
    flex: 1;
    min-width: 0;
    overflow: auto;
  }
  .fhead {
    position: sticky;
    top: 0;
    left: 0;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    font-size: 12px;
    background: var(--bg-elev);
    border-bottom: 1px solid var(--border);
  }
  .note {
    padding: 6px 12px;
    font-size: 12px;
    color: var(--text-2);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .note.err {
    color: var(--danger);
  }
  .code {
    font-family: var(--mono);
    font-size: 12px;
    line-height: 18px;
    min-width: max-content;
    padding-bottom: 12px;
  }
  .hunk {
    padding: 4px 12px;
    margin-top: 6px;
    color: var(--text-3);
    background: var(--bg-hover);
  }
  .ln {
    display: flex;
    width: 100%;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    text-align: left;
    font: inherit;
    color: var(--text);
    cursor: pointer;
  }
  .ln:hover {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .ln.add {
    background: color-mix(in srgb, var(--s-done) 13%, transparent);
  }
  .ln.del {
    background: color-mix(in srgb, var(--danger) 13%, transparent);
  }
  .ln.has {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .no {
    flex: none;
    width: 44px;
    padding-right: 8px;
    text-align: right;
    color: var(--text-3);
    user-select: none;
  }
  .sign {
    flex: none;
    width: 16px;
    color: var(--text-3);
    user-select: none;
  }
  .txt {
    white-space: pre;
    padding-right: 16px;
  }
  .cbox,
  .comment {
    margin: 4px 12px 6px 104px;
    max-width: 640px;
    font-family: var(--font);
  }
  .cbox textarea {
    width: 100%;
    resize: vertical;
    font: 12.5px var(--font);
    padding: 6px 8px;
    border-radius: 6px;
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    color: var(--text);
  }
  .cactions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
  }
  .comment {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-left: 2px solid var(--accent);
    border-radius: 6px;
    background: var(--bg-elev);
    font-size: 12.5px;
    text-align: left;
    color: var(--text);
    white-space: pre-wrap;
    width: calc(100% - 116px);
  }
  .comment :global(svg) {
    flex: none;
    margin-top: 3px;
    color: var(--text-3);
  }
  kbd {
    font-family: var(--font);
    font-size: 10px;
    padding: 0 3px;
    border: 1px solid var(--border-strong);
    border-radius: 3px;
  }
  .review {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
    background: var(--bg-elev);
  }
  .overall {
    flex: 1;
    min-width: 0;
    resize: none;
    font: 12.5px var(--font);
    padding: 5px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text);
    field-sizing: content;
    max-height: 120px;
  }
</style>
