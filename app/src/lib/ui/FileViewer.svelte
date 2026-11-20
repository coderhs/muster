<script lang="ts">
  // A file named in a card's terminal or chat, shown over the card at its line: Quick
  // Look for code. Esc closes it; the buttons hand the file to the user's editor.
  import { onMount, tick } from "svelte";
  import type { CardFile } from "../protocol/types";
  import { errText } from "../protocol/client";
  import { useApp } from "../state/context";
  import { highlightLines, languageFor, MAX_HIGHLIGHT } from "./highlight";
  import { refLabel } from "./fileRef";
  import { bytes } from "./format";
  import Icon from "./Icon.svelte";

  const app = useApp();
  // svelte-ignore state_referenced_locally
  const view = app.fileView!;

  let file = $state<CardFile | null>(null);
  let error = $state<string | null>(null);
  let bodyEl = $state<HTMLDivElement>();
  let findEl = $state<HTMLInputElement>();
  let rootEl = $state<HTMLDivElement>();
  let query = $state("");
  let current = $state(-1);
  let editorLabel = $state<string | null>(null);

  const lines = $derived(file && !file.binary ? highlightLines(file.content, languageFor(file.path)) : []);
  const plainLines = $derived(file && !file.binary ? file.content.replace(/\r\n?/g, "\n").split("\n") : []);
  const matches = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [] as number[];
    const out: number[] = [];
    plainLines.forEach((l, i) => l.toLowerCase().includes(q) && out.push(i));
    return out;
  });
  const matchSet = $derived(new Set(matches));
  const target = view.line && view.line > 0 ? view.line - 1 : -1;
  const canEdit = !!app.platform.openInEditor;

  function close() {
    app.fileView = null;
  }

  async function scrollToLine(i: number, smooth = false) {
    await tick();
    const row = bodyEl?.querySelector<HTMLElement>(`[data-ln="${i}"]`);
    if (row && bodyEl) {
      const top = row.offsetTop - bodyEl.clientHeight / 2 + row.offsetHeight / 2;
      bodyEl.scrollTo({ top: Math.max(0, top), behavior: smooth ? "smooth" : "auto" });
    }
  }

  onMount(() => {
    // Back to where the user was (the terminal, the chat) on closing.
    const before = document.activeElement as HTMLElement | null;
    rootEl?.focus();
    app.client
      .request("card.file", { id: view.cardId, path: view.path })
      .then(async (r) => {
        file = r;
        if (target >= 0) await scrollToLine(target);
      })
      .catch((e) => (error = errText(e)));
    if (app.platform.getEditor && app.platform.listEditors) {
      Promise.all([app.platform.getEditor(), app.platform.listEditors()])
        .then(([pref, list]) => {
          const hit = list.find((e) => e.id === pref.editor);
          editorLabel = pref.editor === "system" ? "default app" : pref.editor === "custom" ? "editor" : (hit?.label ?? null);
        })
        .catch(() => {});
    }
    return () => before?.focus?.();
  });

  function step(dir: 1 | -1) {
    if (!matches.length) return;
    const from = current >= 0 ? current : target;
    let next = matches.find((m) => (dir === 1 ? m > from : false));
    if (dir === -1) next = [...matches].reverse().find((m) => m < from);
    current = next ?? (dir === 1 ? matches[0] : matches[matches.length - 1]);
    void scrollToLine(current, true);
  }

  function openEditor() {
    if (!file || !app.platform.openInEditor) return;
    app.platform.openInEditor(file.absPath, view.line, view.col).catch((e) => app.toast(`Could not open the editor: ${errText(e)}`));
  }

  function reveal() {
    if (!file || !app.platform.revealFile) return;
    app.platform.revealFile(file.absPath).catch((e) => app.toast(`Could not show the file: ${errText(e)}`));
  }

  function copyPath() {
    const text = refLabel({ path: file?.path ?? view.path, line: view.line });
    navigator.clipboard.writeText(text).then(
      () => app.toast(`Copied ${text}`, "info"),
      () => app.toast("Could not copy the path"),
    );
  }

  function onKey(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (document.activeElement === findEl && query) query = "";
      else close();
    } else if (mod && e.key.toLowerCase() === "f") {
      e.preventDefault();
      e.stopPropagation();
      findEl?.focus();
      findEl?.select();
    } else if (mod && e.key.toLowerCase() === "o" && canEdit) {
      e.preventDefault();
      openEditor();
    } else if (e.key === "Enter" && document.activeElement === findEl) {
      e.preventDefault();
      step(e.shiftKey ? -1 : 1);
    }
  }
</script>

<div class="scrim" onclick={close} role="presentation"></div>
<div class="viewer" role="dialog" aria-modal="true" aria-label="File {view.path}" tabindex="-1" bind:this={rootEl} onkeydown={onKey}>
  <header>
    <div class="name" title={file?.absPath ?? view.path}>
      <span class="path">{file?.path ?? view.path}</span>
      {#if view.line}<span class="line">:{view.line}{view.col ? `:${view.col}` : ""}</span>{/if}
    </div>
    <div class="find">
      <Icon name="search" size={12} />
      <input bind:this={findEl} bind:value={query} placeholder="Find" spellcheck="false" oninput={() => (current = -1)} />
      {#if query.trim()}<span class="count">{matches.length ? `${current >= 0 ? matches.indexOf(current) + 1 : 0}/${matches.length}` : "0"}</span>{/if}
    </div>
    <div class="actions">
      {#if canEdit}
        <button class="btn primary" onclick={openEditor} disabled={!file} title="Open in your editor (⌘O); choose it in Settings">
          Open in {editorLabel ?? "editor"}
        </button>
      {/if}
      {#if app.platform.revealFile}
        <button class="btn" onclick={reveal} disabled={!file}>Reveal in Finder</button>
      {/if}
      <button class="btn" onclick={copyPath}>Copy path</button>
      <button class="icon-btn" onclick={close} aria-label="Close" title="Close (Esc)"><Icon name="close" size={14} /></button>
    </div>
  </header>
  <div class="body" bind:this={bodyEl}>
    {#if error}
      <div class="note err"><Icon name="warning" size={13} />{error}</div>
    {:else if !file}
      <div class="note">Loading…</div>
    {:else if file.binary}
      <div class="note">{file.path} is a binary file ({bytes(file.size)}).</div>
    {:else}
      <div class="code" class:plain={file.content.length > MAX_HIGHLIGHT}>
        {#each lines as html, i (i)}
          <div class="row" data-ln={i} class:target={i === target} class:match={matchSet.has(i)} class:current={i === current}>
            <span class="ln">{i + 1}</span><span class="src">{@html html || " "}</span>
          </div>
        {/each}
      </div>
      {#if file.truncated}<div class="note">Only the first {bytes(file.content.length)} of {bytes(file.size)} are shown.</div>{/if}
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 210;
    background: var(--scrim);
  }
  .viewer {
    position: fixed;
    inset: 5vh 5vw;
    z-index: 211;
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border-radius: 12px;
    box-shadow: var(--shadow-pop);
    overflow: hidden;
    outline: none;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px 8px 14px;
    border-bottom: 1px solid var(--border);
    flex: none;
    flex-wrap: wrap;
  }
  .name {
    flex: 1;
    min-width: 120px;
    font: 12.5px var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .name .path,
  .name .line {
    direction: ltr;
    unicode-bidi: embed;
  }
  .line {
    color: var(--text-3);
  }
  .find {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    height: 26px;
    border-radius: 6px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    color: var(--text-3);
  }
  .find input {
    width: 140px;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 12px;
  }
  .count {
    font-size: 11px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--bg-term);
    -webkit-user-select: text;
    user-select: text;
  }
  .note {
    padding: 16px;
    color: var(--text-3);
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .note.err {
    color: var(--danger);
  }
  .code {
    font: 12px/1.55 var(--mono);
    padding: 6px 0 24px;
    min-width: max-content;
  }
  .row {
    display: flex;
    white-space: pre;
  }
  .ln {
    position: sticky;
    left: 0;
    flex: none;
    width: 56px;
    padding-right: 14px;
    text-align: right;
    color: var(--text-3);
    background: var(--bg-term);
    -webkit-user-select: none;
    user-select: none;
  }
  .src {
    padding-right: 24px;
  }
  .row.target,
  .row.target .ln {
    background: color-mix(in srgb, var(--accent) 22%, var(--bg-term));
  }
  .row.target .ln {
    color: var(--text);
  }
  .row.match .src {
    background: color-mix(in srgb, var(--s-needs) 22%, transparent);
  }
  .row.current .src {
    background: color-mix(in srgb, var(--s-needs) 45%, transparent);
  }

  /* highlight.js classes: one palette for dark, one for light. */
  .code :global(.hljs-comment),
  .code :global(.hljs-quote) {
    color: #7f848e;
    font-style: italic;
  }
  .code :global(.hljs-keyword),
  .code :global(.hljs-selector-tag),
  .code :global(.hljs-meta .hljs-keyword) {
    color: #c678dd;
  }
  .code :global(.hljs-string),
  .code :global(.hljs-regexp),
  .code :global(.hljs-addition),
  .code :global(.hljs-meta .hljs-string) {
    color: #98c379;
  }
  .code :global(.hljs-number),
  .code :global(.hljs-literal),
  .code :global(.hljs-symbol),
  .code :global(.hljs-variable.constant_) {
    color: #d19a66;
  }
  .code :global(.hljs-title),
  .code :global(.hljs-title.function_),
  .code :global(.hljs-section) {
    color: #61afef;
  }
  .code :global(.hljs-type),
  .code :global(.hljs-title.class_),
  .code :global(.hljs-built_in) {
    color: #e5c07b;
  }
  .code :global(.hljs-attr),
  .code :global(.hljs-attribute),
  .code :global(.hljs-property),
  .code :global(.hljs-variable),
  .code :global(.hljs-template-variable),
  .code :global(.hljs-name) {
    color: #e06c75;
  }
  .code :global(.hljs-meta),
  .code :global(.hljs-selector-class),
  .code :global(.hljs-selector-id) {
    color: #56b6c2;
  }
  .code :global(.hljs-deletion) {
    color: #e06c75;
  }
  .code :global(.hljs-emphasis) {
    font-style: italic;
  }
  .code :global(.hljs-strong) {
    font-weight: 600;
  }
  @media (prefers-color-scheme: light) {
    .code :global(.hljs-comment),
    .code :global(.hljs-quote) {
      color: #8a8f98;
    }
    .code :global(.hljs-keyword),
    .code :global(.hljs-selector-tag),
    .code :global(.hljs-meta .hljs-keyword) {
      color: #a626a4;
    }
    .code :global(.hljs-string),
    .code :global(.hljs-regexp),
    .code :global(.hljs-addition),
    .code :global(.hljs-meta .hljs-string) {
      color: #50a14f;
    }
    .code :global(.hljs-number),
    .code :global(.hljs-literal),
    .code :global(.hljs-symbol),
    .code :global(.hljs-variable.constant_) {
      color: #986801;
    }
    .code :global(.hljs-title),
    .code :global(.hljs-title.function_),
    .code :global(.hljs-section) {
      color: #4078f2;
    }
    .code :global(.hljs-type),
    .code :global(.hljs-title.class_),
    .code :global(.hljs-built_in) {
      color: #c18401;
    }
    .code :global(.hljs-attr),
    .code :global(.hljs-attribute),
    .code :global(.hljs-property),
    .code :global(.hljs-variable),
    .code :global(.hljs-template-variable),
    .code :global(.hljs-name),
    .code :global(.hljs-deletion) {
      color: #e45649;
    }
    .code :global(.hljs-meta),
    .code :global(.hljs-selector-class),
    .code :global(.hljs-selector-id) {
      color: #0184bc;
    }
  }
</style>
