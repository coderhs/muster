<script lang="ts">
  import { useApp } from "../state/context";
  import Popover from "./Popover.svelte";
  import AgentMark from "./AgentMark.svelte";

  const app = useApp();
  // svelte-ignore state_referenced_locally
  const q = app.quick!;
  const last = $derived(app.lastAgent(app.currentProjectId));
  // Last used first, then installed agents in daemon order; uninstalled ones are left out.
  const agents = $derived.by(() => {
    const list = app.agentsWithShell.filter((a) => a.installed);
    const i = list.findIndex((a) => a.id === last);
    return i > 0 ? [list[i], ...list.slice(0, i), ...list.slice(i + 1)] : list;
  });
  const column = $derived(app.board.columns[app.defaultColumn(q.columnId) ?? ""]);
  let busy = $state(false);

  async function choose(agentId: string) {
    if (busy) return;
    busy = true;
    const columnId = q.columnId;
    app.quick = null;
    try {
      await app.quickStart(agentId, columnId);
    } catch {
      /* toast shown */
    } finally {
      busy = false;
    }
  }

  function more() {
    const columnId = q.columnId;
    app.quick = null;
    app.newCard = { columnId };
  }

  function onKey(e: KeyboardEvent) {
    if (/^[1-9]$/.test(e.key) && !e.metaKey && !e.ctrlKey) {
      const a = agents[Number(e.key) - 1];
      if (a) {
        e.preventDefault();
        void choose(a.id);
      }
    } else if (e.key === "m" || e.key === "o") {
      e.preventDefault();
      more();
    }
  }
</script>

<Popover anchor={q.anchor} onclose={() => (app.quick = null)} width={236}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div onkeydown={onKey}>
    <div class="sub">Start a session{column ? ` in ${column.title}` : ""}</div>
    {#each agents as a, i (a.id)}
      <button class="mi" disabled={busy} onclick={() => choose(a.id)}>
        <AgentMark agent={a} size={16} />
        {a.label}
        <span class="hint">{a.id === last ? "last · " : ""}{i < 9 ? i + 1 : ""}</span>
      </button>
    {/each}
    <div class="sep"></div>
    <button class="mi" onclick={more}>More options…<span class="hint">M</span></button>
  </div>
</Popover>
