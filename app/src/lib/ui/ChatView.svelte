<script lang="ts">
  import { onMount, tick } from "svelte";
  import type { Card, ChatMessage } from "../protocol/types";
  import { errText } from "../protocol/client";
  import { useApp } from "../state/context";
  import { composeInput, mergeChat } from "../agent/agent";
  import Icon from "./Icon.svelte";
  import ChatMarkdown from "./ChatMarkdown.svelte";

  let { card, active }: { card: Card; active: boolean } = $props();
  const app = useApp();
  // svelte-ignore state_referenced_locally
  const cardId = card.id;

  let messages = $state.raw<ChatMessage[]>([]);
  let older = $state<string | null>(null);
  let loaded = $state(false);
  let loadingOlder = $state(false);
  let error = $state<string | null>(null);
  let listEl = $state<HTMLDivElement>();
  let draft = $state("");
  let sending = $state(false);
  let stickUntil = 0;

  const nearBottom = () => !listEl || listEl.scrollHeight - listEl.scrollTop - listEl.clientHeight < 80;

  async function scrollToBottom() {
    await tick();
    if (listEl) listEl.scrollTop = listEl.scrollHeight;
  }

  async function load() {
    error = null;
    try {
      const r = await app.client.request("chat.page", { cardId, limit: 50 });
      messages = r.messages;
      older = r.older ?? null;
      loaded = true;
      await scrollToBottom();
    } catch (e) {
      error = `Could not load the conversation: ${errText(e)}`;
    }
  }

  async function loadOlder() {
    if (!older || loadingOlder) return;
    loadingOlder = true;
    const before = listEl?.scrollHeight ?? 0;
    try {
      const r = await app.client.request("chat.page", { cardId, before: older, limit: 50 });
      messages = mergeChat(messages, r.messages, "prepend");
      older = r.older ?? null;
      await tick();
      // Keep the message the user was looking at in place.
      if (listEl) listEl.scrollTop += listEl.scrollHeight - before;
    } catch (e) {
      error = `Could not load older messages: ${errText(e)}`;
    } finally {
      loadingOlder = false;
    }
  }

  onMount(() => {
    const offAppend = app.client.on("chat.appended", (p) => {
      if (p.cardId !== cardId) return;
      const stick = nearBottom() || Date.now() < stickUntil;
      messages = mergeChat(messages, p.messages, "append");
      if (stick) void scrollToBottom();
    });
    // The subscription lives on the connection: page again after every reconnect.
    const offReady = app.client.onReady(() => void load());
    return () => {
      offAppend();
      offReady();
      if (app.client.ready) app.client.request("chat.unsubscribe", { cardId }).catch(() => {});
    };
  });

  $effect(() => {
    if (active && loaded) void scrollToBottom();
  });

  async function send() {
    const data = composeInput(draft);
    if (!data || sending) return;
    sending = true;
    try {
      // Enter goes separately, after the agent has taken the text: arriving in the same
      // write, Claude Code reads the whole burst as a paste and Enter as a new line.
      await app.client.request("term.write", { cardId, data });
      await new Promise((r) => setTimeout(r, SUBMIT_DELAY_MS));
      await app.client.request("term.write", { cardId, data: "\r" });
      draft = "";
      // Follow the conversation after sending, wherever the user had scrolled to.
      stickUntil = Date.now() + 5000;
      void scrollToBottom();
    } catch (e) {
      app.toast(`Message not sent: ${errText(e)}`);
    } finally {
      sending = false;
    }
  }

  const SUBMIT_DELAY_MS = 150;

  const time = (t: number | null | undefined) =>
    t ? new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }) : "";
</script>

<div class="chat">
  <div class="list" bind:this={listEl} onscroll={() => listEl && listEl.scrollTop < 60 && void loadOlder()}>
    {#if older}
      <button class="older" onclick={loadOlder} disabled={loadingOlder}>{loadingOlder ? "Loading…" : "Load earlier messages"}</button>
    {:else if loaded && messages.length}
      <div class="start">Start of conversation</div>
    {/if}
    {#if error}<div class="err"><Icon name="warning" size={13} />{error}</div>{/if}
    {#if loaded && !messages.length && !error}
      <div class="empty">No messages yet. The conversation appears here as the agent works.</div>
    {/if}
    {#each messages as m (m.id)}
      <div class="msg r-{m.role}">
        {#each m.parts as part, i (i)}
          {#if part.type === "text"}
            {#if m.role === "assistant"}
              <div class="text md"><ChatMarkdown text={part.text} /></div>
            {:else}
              <div class="text">{part.text}</div>
            {/if}
          {:else if part.type === "thinking"}
            <div class="chip think">Thinking…</div>
          {:else if part.type === "toolCall"}
            <div class="chip tool" title={part.summary}><Icon name="terminal" size={12} /><b>{part.name}</b> {part.summary}</div>
          {:else if part.type === "toolResult"}
            <div class="chip result" class:error={part.isError} title={part.summary}>
              <Icon name={part.isError ? "warning" : "check"} size={12} />{part.summary || (part.isError ? "Error" : "Done")}
            </div>
          {:else if part.type === "image"}
            <div class="chip">Image</div>
          {:else if part.type === "interrupted"}
            <div class="chip error">Interrupted</div>
          {/if}
        {/each}
        {#if m.timestamp && m.role !== "tool"}<div class="ts">{time(m.timestamp)}</div>{/if}
      </div>
    {/each}
  </div>
  <form
    class="composer"
    onsubmit={(e) => {
      e.preventDefault();
      void send();
    }}
  >
    <textarea
      class="field"
      rows="2"
      placeholder={card.hasSession ? "Message the agent (Enter sends, Shift+Enter new line)" : "No session running"}
      disabled={!card.hasSession}
      bind:value={draft}
      onkeydown={(e) => {
        if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
          e.preventDefault();
          void send();
        }
      }}
    ></textarea>
    <button class="btn primary" type="submit" disabled={!card.hasSession || !draft.trim() || sending}>Send</button>
  </form>
</div>

<style>
  .chat {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-term);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    -webkit-user-select: text;
    user-select: text;
  }
  .older,
  .start {
    align-self: center;
    font-size: 11.5px;
    color: var(--text-3);
    border: 0;
    background: transparent;
  }
  .older:hover {
    color: var(--accent);
  }
  .empty {
    margin: auto;
    color: var(--text-3);
  }
  .err {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--danger);
    font-size: 12px;
  }
  .msg {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-width: 86%;
  }
  .r-user {
    align-self: flex-end;
    align-items: flex-end;
  }
  .r-user .text {
    background: color-mix(in srgb, var(--accent) 18%, var(--bg-elev));
    padding: 7px 11px;
    border-radius: 12px 12px 3px 12px;
  }
  .r-assistant,
  .r-tool {
    align-self: flex-start;
  }
  .text.md {
    white-space: normal;
  }
  .text {
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.5;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    max-width: 100%;
    font-size: 11.5px;
    font-family: var(--mono);
    color: var(--text-2);
    background: var(--bg-hover);
    border-radius: 6px;
    padding: 3px 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip b {
    color: var(--text);
    font-weight: 600;
  }
  .think {
    font-family: var(--font);
    font-style: italic;
  }
  .chip.error {
    color: var(--danger);
  }
  .ts {
    font-size: 10.5px;
    color: var(--text-3);
  }
  .composer {
    display: flex;
    gap: 8px;
    align-items: flex-end;
    padding: 10px 12px;
    border-top: 1px solid var(--border);
    background: var(--bg-elev);
  }
  .composer textarea {
    resize: none;
    min-height: 0;
  }
</style>
