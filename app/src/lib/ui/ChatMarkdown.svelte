<script lang="ts">
  // An agent's reply rendered as Markdown. The text is parsed into a tree and rendered
  // as elements, never inserted as HTML. Web links open in the browser; file paths
  // (`src/lib.rs:12`, in code, links or plain text) open in the file viewer.
  import { errText } from "../protocol/client";
  import { useApp } from "../state/context";
  import { parse, target, type Block, type Inline } from "./markdown";
  import { findRefs, parseRef, type FileRef } from "./fileRef";

  let { text, cardId }: { text: string; cardId: string } = $props();
  const app = useApp();
  // A reply is never front matter: keep a leading rule from being read as one.
  const blocks = $derived(parse(text.startsWith("---") ? `\n${text}` : text));

  function follow(e: MouseEvent, href: string) {
    e.preventDefault();
    const t = target(href, "");
    if (t.kind === "web") void app.platform.openUrl(t.url).catch((err) => app.toast(`Could not open the link: ${errText(err)}`));
    else {
      const ref = linkRef(href);
      if (ref) show(ref);
    }
  }

  /** A link to a file (`src/a.rs`, `src/a.rs#L12`, `/abs/a.rs:3`), or null. */
  function linkRef(href: string): FileRef | null {
    if (/^[a-z][a-z0-9+.-]*:/i.test(href) && !href.startsWith("file:")) return null;
    let h = href.replace(/^file:\/\//, "");
    try {
      h = decodeURI(h);
    } catch {
      /* as written */
    }
    return parseRef(h) ?? (h && !h.startsWith("#") ? { path: h.split("#")[0] } : null);
  }

  function show(ref: FileRef) {
    app.fileView = { cardId, ...ref };
  }

  /** Plain text cut into text and file references. */
  function pieces(s: string): ({ s: string } | { s: string; ref: FileRef })[] {
    const found = findRefs(s);
    if (!found.length) return [{ s }];
    const out: ({ s: string } | { s: string; ref: FileRef })[] = [];
    let at = 0;
    for (const r of found) {
      if (r.start > at) out.push({ s: s.slice(at, r.start) });
      out.push({ s: s.slice(r.start, r.end), ref: { path: r.path, line: r.line, col: r.col } });
      at = r.end;
    }
    if (at < s.length) out.push({ s: s.slice(at) });
    return out;
  }
</script>

{#snippet inl(nodes: Inline[])}{#each nodes as n}{#if n.t === "text"}{#each pieces(n.s) as p}{#if "ref" in p}<button type="button" class="file" title="Show {p.s}" onclick={() => show(p.ref)}>{p.s}</button>{:else}{p.s}{/if}{/each}{:else if n.t === "code"}{@const ref = parseRef(n.s)}{#if ref}<button type="button" class="file code" title="Show {n.s}" onclick={() => show(ref)}><code>{n.s}</code></button>{:else}<code>{n.s}</code>{/if}{:else if n.t === "strong"}<strong>{@render inl(n.c)}</strong>{:else if n.t === "em"}<em>{@render inl(n.c)}</em>{:else if n.t === "del"}<del>{@render inl(n.c)}</del>{:else if n.t === "link"}{#if target(n.href, "").kind === "web" || linkRef(n.href)}<a href={n.href} title={n.href} onclick={(e) => follow(e, n.href)}>{@render inl(n.c)}</a>{:else}<span class="ref" title={n.href}>{@render inl(n.c)}</span>{/if}{:else}<br />{/if}{/each}{/snippet}

{#snippet body(bs: Block[])}
  {#each bs as b}
    {#if b.t === "heading"}
      <svelte:element this={`h${Math.min(b.level + 2, 6)}`}>{@render inl(b.c)}</svelte:element>
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

<div class="md">{@render body(blocks)}</div>

<style>
  .md {
    min-width: 0;
    line-height: 1.5;
    overflow-wrap: break-word;
  }
  .md > :global(:first-child) {
    margin-top: 0;
  }
  .md > :global(:last-child) {
    margin-bottom: 0;
  }
  .md :is(h3, h4, h5, h6) {
    margin: 0.9em 0 0.35em;
    line-height: 1.3;
    font-weight: 600;
  }
  .md h3 {
    font-size: 14.5px;
  }
  .md h4 {
    font-size: 13.5px;
  }
  .md :is(h5, h6) {
    font-size: 13px;
    color: var(--text-2);
  }
  .md p {
    margin: 0 0 0.6em;
  }
  .md a {
    color: var(--accent);
    text-decoration: none;
    cursor: pointer;
  }
  .md a:hover {
    text-decoration: underline;
  }
  .file {
    all: unset;
    color: var(--accent);
    cursor: pointer;
    overflow-wrap: anywhere;
  }
  .file:hover {
    text-decoration: underline;
  }
  .file:focus-visible {
    outline: 2px solid var(--accent);
    border-radius: 3px;
  }
  .file.code :global(code) {
    color: var(--accent);
  }
  /* Inline code only; a block's code is styled under pre. */
  .md :not(pre) > code {
    font: 0.9em var(--mono);
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--bg-hover);
  }
  .md pre {
    margin: 0 0 0.7em;
    padding: 8px 10px;
    border-radius: var(--radius);
    background: var(--bg-input);
    border: 1px solid var(--border);
    overflow-x: auto;
    line-height: 1.45;
  }
  .md pre code {
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre;
  }
  .md blockquote {
    margin: 0 0 0.7em;
    padding: 1px 0 1px 10px;
    border-left: 3px solid var(--border-strong);
    color: var(--text-2);
  }
  .md blockquote > :global(:last-child),
  .md li > :global(:last-child) {
    margin-bottom: 0;
  }
  .md :is(ul, ol) {
    margin: 0 0 0.6em;
    padding-left: 20px;
  }
  .md li {
    margin: 2px 0;
  }
  .md li > p {
    margin-bottom: 0.25em;
  }
  .md li.task {
    list-style: none;
    position: relative;
  }
  .md li.task > input {
    position: absolute;
    left: -18px;
    top: 4px;
    margin: 0;
  }
  .md hr {
    border: 0;
    border-top: 1px solid var(--border-strong);
    margin: 0.9em 0;
  }
  .table {
    margin: 0 0 0.7em;
    overflow-x: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th,
  td {
    padding: 4px 9px;
    border: 1px solid var(--border);
    text-align: left;
    vertical-align: top;
  }
  th {
    background: var(--bg-hover);
    font-weight: 600;
  }
</style>
