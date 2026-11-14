<script lang="ts">
  // The comments of one card: yours and the agents'. The daemon stamps each author.
  import { tick } from "svelte";
  import type { Comment, Id } from "../protocol/types";
  import { useApp } from "../state/context";
  import { clock } from "../state/clock.svelte";
  import AgentMark from "./AgentMark.svelte";
  import Icon from "./Icon.svelte";
  import { authorLabel } from "./authors";
  import { agoLabel } from "./time";

  let { cardId, active }: { cardId: Id; active: boolean } = $props();
  const app = useApp();

  let comments = $state.raw<Comment[]>([]);
  let loaded = $state(false);
  let draft = $state("");
  let posting = $state(false);
  let list = $state<HTMLDivElement>();
  let box = $state<HTMLTextAreaElement>();

  async function scrollToEnd() {
    await tick();
    if (list) list.scrollTop = list.scrollHeight;
  }

  // (Re)load on mount and after every reconnect; changes arrive as events in between.
  $effect(() => {
    void app.epoch;
    const id = cardId;
    app.client
      .request("comment.list", { cardId: id })
      .then((r) => {
        if (id !== cardId) return;
        comments = r.comments;
        loaded = true;
        void scrollToEnd();
      })
      .catch(() => {});
  });

  $effect(() =>
    app.client.on("comment.changed", (p) => {
      if ("removedId" in p) {
        if (p.cardId === cardId) comments = comments.filter((c) => c.id !== p.removedId);
      } else if (p.comment.cardId === cardId && !comments.some((c) => c.id === p.comment.id)) {
        const atEnd = !list || list.scrollHeight - list.scrollTop - list.clientHeight < 40;
        comments = [...comments, p.comment];
        if (atEnd) void scrollToEnd();
      }
    }),
  );

  $effect(() => {
    if (active) void scrollToEnd();
  });

  async function post() {
    const text = draft.trim();
    if (!text || posting) return;
    posting = true;
    try {
      await app.call("comment.add", { cardId, text });
      draft = "";
      void scrollToEnd();
    } catch {
      /* toast shown; the draft stays */
    } finally {
      posting = false;
      box?.focus();
    }
  }

  function remove(id: Id) {
    void app.call("comment.delete", { id }).catch(() => {});
  }
</script>

<div class="comments">
  <div class="list" bind:this={list}>
    {#each comments as c (c.id)}
      <article class="c" class:agent={c.author.kind === "agent"}>
        <header>
          {#if c.author.kind === "agent" && c.author.agentId}
            <AgentMark agent={app.agent(c.author.agentId)} size={16} />
          {:else}
            <span class="you" aria-hidden="true"></span>
          {/if}
          <span class="who" title={c.author.kind === "agent" ? `Written by the agent in card “${c.author.cardTitle ?? ""}”` : "Written by you"}>
            {authorLabel(c.author, (id) => app.agent(id), cardId)}
          </span>
          <time title={new Date(c.createdAt).toLocaleString()}>{agoLabel(c.createdAt, clock.now)}</time>
          <button class="icon-btn del" title="Delete comment" onclick={() => remove(c.id)}><Icon name="trash" size={12} /></button>
        </header>
        <p>{c.text}</p>
      </article>
    {:else}
      {#if loaded}
        <div class="empty">No comments yet. You and the card's agent can both write here; agents use <code>muster comment add</code>.</div>
      {/if}
    {/each}
  </div>
  <form
    class="compose"
    onsubmit={(e) => {
      e.preventDefault();
      void post();
    }}
  >
    <textarea
      class="field"
      rows="3"
      placeholder="Write a comment"
      aria-label="New comment"
      bind:this={box}
      bind:value={draft}
      onkeydown={(e) => {
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          void post();
        }
      }}
    ></textarea>
    <div class="row">
      <span class="hint"><kbd>⌘</kbd><kbd>↵</kbd> to post</span>
      <button class="btn small primary" type="submit" disabled={!draft.trim() || posting}>Comment</button>
    </div>
  </form>
</div>

<style>
  .comments {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0 -4px;
    padding: 0 4px;
  }
  .c {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 7px 9px 8px;
    background: var(--bg-hover);
  }
  .c.agent {
    background: transparent;
  }
  header {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 20px;
  }
  .you {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--accent, #0a84ff);
    flex: none;
  }
  .who {
    font-size: 12px;
    font-weight: 600;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  time {
    font-size: 11px;
    color: var(--text-3);
    white-space: nowrap;
    margin-right: auto;
  }
  .del {
    visibility: hidden;
    width: 20px;
    height: 20px;
  }
  .c:hover .del,
  .del:focus-visible {
    visibility: visible;
  }
  p {
    margin: 4px 0 0;
    font-size: 12.5px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .empty {
    font-size: 12px;
    color: var(--text-3);
    line-height: 1.45;
  }
  code {
    font-family: var(--mono);
    font-size: 11.5px;
  }
  .compose {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  textarea {
    resize: none;
    height: auto;
    padding: 6px 8px;
    line-height: 1.4;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .hint {
    font-size: 11px;
    color: var(--text-3);
  }
</style>
