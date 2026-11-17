<script lang="ts">
  // An agent's reply rendered as Markdown. The text is parsed into a tree and rendered
  // as elements, never inserted as HTML. Only web links open (in the browser).
  import { errText } from "../protocol/client";
  import { useApp } from "../state/context";
  import { parse, target, type Block, type Inline } from "./markdown";

  let { text }: { text: string } = $props();
  const app = useApp();
  // A reply is never front matter: keep a leading rule from being read as one.
  const blocks = $derived(parse(text.startsWith("---") ? `\n${text}` : text));

  function follow(e: MouseEvent, href: string) {
    e.preventDefault();
    const t = target(href, "");
    if (t.kind === "web") void app.platform.openUrl(t.url).catch((err) => app.toast(`Could not open the link: ${errText(err)}`));
  }
</script>

{#snippet inl(nodes: Inline[])}{#each nodes as n}{#if n.t === "text"}{n.s}{:else if n.t === "code"}<code>{n.s}</code>{:else if n.t === "strong"}<strong>{@render inl(n.c)}</strong>{:else if n.t === "em"}<em>{@render inl(n.c)}</em>{:else if n.t === "del"}<del>{@render inl(n.c)}</del>{:else if n.t === "link"}{#if target(n.href, "").kind === "web"}<a href={n.href} title={n.href} onclick={(e) => follow(e, n.href)}>{@render inl(n.c)}</a>{:else}<span class="ref" title={n.href}>{@render inl(n.c)}</span>{/if}{:else}<br />{/if}{/each}{/snippet}

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
