<script lang="ts">
  import { useApp } from "../state/context";
  import Popover from "./Popover.svelte";
  import Icon from "./Icon.svelte";
  import AgentMark from "./AgentMark.svelte";

  const app = useApp();
  // Captured once: the menu closes (and app.cardMenu becomes null) before actions run.
  // svelte-ignore state_referenced_locally
  const m = app.cardMenu!;
  const card = $derived(app.card(m.cardId));
  const agents = $derived.by(() => {
    const list = app.agentsWithShell.filter((a) => a.installed && a.id !== "shell");
    const i = list.findIndex((a) => a.id === card?.agentId);
    return i > 0 ? [list[i], ...list.slice(0, i), ...list.slice(i + 1)] : list;
  });
  const close = () => (app.cardMenu = null);

  function act(f: () => unknown) {
    close();
    try {
      void Promise.resolve(f()).catch(() => {});
    } catch {
      /* toast shown */
    }
  }
</script>

{#if card}
  <Popover anchor={m.anchor} onclose={close} width={220}>
    <button class="mi" onclick={() => act(() => app.openCard(card.id))}>Open<span class="hint">↵</span></button>
    {#if card.restorable}
      <button class="mi" onclick={() => act(() => app.startCard(card.id))}><Icon name="restore" size={14} />Restore session</button>
    {:else if !card.hasSession}
      <button class="mi" onclick={() => act(() => app.startCard(card.id))}><Icon name="play" size={14} />Start</button>
    {/if}
    <button class="mi" onclick={() => act(() => (app.linking = { cardId: card.id, anchor: m.anchor }))}><Icon name="link" size={14} />Link to card…</button>
    {#if agents.length}
      <div class="sep"></div>
      <div class="sub">Hand off to</div>
      {#each agents as a (a.id)}
        <button class="mi" onclick={() => act(() => app.handoff(card.id, a.id))}><AgentMark agent={a} size={14} />{a.label}</button>
      {/each}
    {/if}
  </Popover>
{/if}
