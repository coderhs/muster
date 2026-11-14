<script lang="ts">
  import { useApp } from "../state/context";
  import Popover from "./Popover.svelte";
  import AgentMark from "./AgentMark.svelte";

  const app = useApp();
  // svelte-ignore state_referenced_locally
  const l = app.linking!;
  let q = $state("");
  const linked = $derived(new Set(app.linkedTo(l.cardId)));
  const projectName = (id: string) => app.projects.find((p) => p.id === id)?.name ?? "";
  const candidates = $derived(
    Object.values(app.allCards)
      .filter((c) => c.id !== l.cardId && !linked.has(c.id))
      .filter((c) => !q.trim() || `${c.title} ${projectName(c.projectId)} ${c.branch ?? ""}`.toLowerCase().includes(q.trim().toLowerCase()))
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .slice(0, 12),
  );

  async function pick(id: string) {
    const from = l.cardId;
    app.linking = null;
    await app.setLinked(from, id, true).catch(() => {});
  }
</script>

<Popover anchor={l.anchor} onclose={() => (app.linking = null)} width={320}>
  <input
    class="field"
    placeholder="Link to which card?"
    bind:value={q}
    onkeydown={(e) => {
      if (e.key === "Enter" && candidates[0]) {
        e.preventDefault();
        void pick(candidates[0].id);
      }
    }}
  />
  <div class="sub">Linked agents can read each other's transcript, screen and notes.</div>
  {#each candidates as c (c.id)}
    <button class="mi" onclick={() => pick(c.id)}>
      <AgentMark agent={app.agent(c.agentId)} size={14} />
      <span class="t">{c.title}</span>
      <span class="hint">{projectName(c.projectId)}</span>
    </button>
  {:else}
    <div class="sub">No other cards{q ? " match" : ""}.</div>
  {/each}
</Popover>

<style>
  .t {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
</style>
