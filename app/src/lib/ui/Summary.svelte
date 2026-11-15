<script lang="ts">
  import { untrack } from "svelte";
  import type { Id, ProjectSummary, SummaryCard } from "../protocol/types";
  import { useApp } from "../state/context";
  import { clock } from "../state/clock.svelte";
  import { errText } from "../protocol/client";
  import { timerLabel } from "../tracker/tracker";
  import { RANGES, RANGE_LABEL, activeNow, dayLabel, endedLabel, rangeOf, runsLabel, splitWorked, timeSpan, workDuration, type RangeKey } from "../summary/summary";
  import AgentMark from "./AgentMark.svelte";

  const app = useApp();
  const projectId = $derived(app.currentProjectId);
  const ready = $derived(app.conn.kind === "ready");

  const RANGE_KEY = "muster.summary.range";
  let range = $state<RangeKey>(recall());
  let data = $state.raw<ProjectSummary | null>(null);
  let error = $state("");
  let loading = $state(false);
  let loads = 0;
  let shownFor: Id | null = null;

  function recall(): RangeKey {
    try {
      const v = localStorage.getItem(RANGE_KEY);
      return RANGES.find((r) => r === v) ?? "today";
    } catch {
      return "today";
    }
  }
  function pick(r: RangeKey) {
    range = r;
    try {
      localStorage.setItem(RANGE_KEY, r);
    } catch {
      /* ignore */
    }
  }

  // Read when the tab opens, when the project or the range changes and when the daemon
  // comes back; after that only on Refresh. "Active now" needs no reading: it comes
  // from the cards the app already keeps current.
  $effect(() => {
    const [pid, r] = [projectId, range];
    void app.epoch;
    if (!pid || !ready) return;
    untrack(() => void load(pid, r));
  });

  async function load(pid: Id, r: RangeKey) {
    const n = ++loads;
    // Another project's numbers must not stay on screen under this one's name.
    if (pid !== shownFor) data = null;
    shownFor = pid;
    loading = true;
    error = "";
    try {
      const { from, to, splits } = rangeOf(r, Date.now());
      const res = await app.client.request("project.summary", { projectId: pid, from, to, splits });
      if (n !== loads) return;
      data = res;
    } catch (e) {
      if (n !== loads) return;
      data = null;
      error = errText(e);
    } finally {
      if (n === loads) loading = false;
    }
  }

  const active = $derived(projectId ? activeNow(Object.values(app.allCards), projectId) : []);
  const parts = $derived(splitWorked(data?.cards ?? []));
  // A range of several days is shown day by day; the daemon leaves out the empty days.
  const days = $derived((data?.days ?? []).map((d) => ({ ...d, worked: d.cards.filter((c) => c.runs > 0) })).filter((d) => d.worked.length));
</script>

<div class="sum">
  <div class="head">
    <div class="ranges" role="tablist" aria-label="Range">
      {#each RANGES as r (r)}
        <button role="tab" aria-selected={range === r} class:on={range === r} onclick={() => pick(r)}>{RANGE_LABEL[r]}</button>
      {/each}
    </div>
    <button class="btn small" disabled={!ready || loading || !projectId} onclick={() => projectId && void load(projectId, range)} title="Read the summary again">Refresh</button>
  </div>

  {#if data}
    {@const t = data.totals}
    <div class="totals">
      <div class="stat" class:zero={!t.cards}><b>{t.cards}</b> card{t.cards === 1 ? "" : "s"} worked on</div>
      <div class="stat" class:zero={!t.workingMs}><b>{workDuration(t.workingMs)}</b> agents working</div>
      <div class="stat fin" class:zero={!t.finished}><b>{t.finished}</b> finished</div>
      <div class="stat" class:zero={!t.created}><b>{t.created}</b> created</div>
    </div>
  {/if}

  {#if active.length}
    <section class="group">
      <h2><span class="gdot"></span>Active now <span class="gcount">{active.length}</span></h2>
      <div class="rows">
        {#each active as c (c.id)}
          <button class="row" onclick={() => void app.openCard(c.id)}>
            <AgentMark agent={app.agent(c.agentId)} size={20} />
            <span class="main">
              <span class="title">{c.title || "Untitled"}</span>
              <span class="meta">
                <span>{app.agent(c.agentId).label}</span>
                {#if c.model}<span class="model" title="Model">{c.model}</span>{/if}
                {#if app.board.columns[c.columnId]}<span class="col">{app.board.columns[c.columnId].title}</span>{/if}
              </span>
            </span>
            <span class="side"><span class="ended t-{c.status === 'needs_you' ? 'needs' : 'running'}">{timerLabel(c, clock.now)}</span></span>
          </button>
        {/each}
      </div>
    </section>
  {/if}

  {#if error}
    <div class="empty err">{error}</div>
  {:else if !data}
    <div class="empty">{projectId ? "Reading the activity log…" : "Pick a project to see its summary."}</div>
  {:else if !parts.worked.length}
    <div class="empty">Nothing was worked on in this range.</div>
  {:else if days.length}
    {#each days as d (d.from)}
      <section class="group">
        <h2>
          {dayLabel(d.from, clock.now)}
          <span class="gcount">{d.totals.cards} card{d.totals.cards === 1 ? "" : "s"} · {workDuration(d.totals.workingMs)}{d.totals.finished ? ` · ${d.totals.finished} finished` : ""}</span>
        </h2>
        <div class="rows">
          {#each d.worked as c (c.id)}{@render row(c)}{/each}
        </div>
      </section>
    {/each}
  {:else}
    <section class="group">
      <h2>Worked on <span class="gcount">{parts.worked.length}</span></h2>
      <div class="rows">
        {#each parts.worked as c (c.id)}{@render row(c)}{/each}
      </div>
    </section>
  {/if}

  {#if data && parts.createdOnly.length}
    <details class="group">
      <summary>Created, not started <span class="gcount">{parts.createdOnly.length}</span></summary>
      <div class="rows">
        {#each parts.createdOnly as c (c.id)}{@render row(c)}{/each}
      </div>
    </details>
  {/if}
</div>

{#snippet row(c: SummaryCard)}
  {@const e = endedLabel(c)}
  <button class="row" onclick={() => void app.openCard(c.id)}>
    <AgentMark agent={app.agent(c.agentId)} size={20} />
    <span class="main">
      <span class="title">{c.title || "Untitled"}</span>
      <span class="meta">
        <span>{app.agent(c.agentId).label}</span>
        {#if c.model}<span class="model" title="Model">{c.model}</span>{/if}
        <span class="col">{c.columnTitle}</span>
        <span class="when">{c.runs ? timeSpan(c.firstAt, c.lastAt) : new Date(c.firstAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}</span>
        {#if c.created && c.runs}<span>new</span>{/if}
      </span>
      {#if c.summary}<span class="text">{c.summary}</span>{/if}
    </span>
    <span class="side">
      {#if c.runs}<span class="work"><b>{workDuration(c.workingMs)}</b> · {runsLabel(c.runs)}</span>{/if}
      <span class="ended t-{e.tone}">{e.text}</span>
    </span>
  </button>
{/snippet}

<style>
  .sum {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 16px 24px;
    max-width: 920px;
    width: 100%;
    margin: 0 auto;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 4px 0 10px;
    flex-wrap: wrap;
  }
  .ranges {
    display: inline-flex;
    background: var(--bg-hover);
    border-radius: 7px;
    padding: 2px;
    gap: 2px;
    max-width: 100%;
    overflow-x: auto;
  }
  .ranges button {
    border: 0;
    background: transparent;
    height: 22px;
    padding: 0 10px;
    border-radius: 5px;
    font-size: 12px;
    color: var(--text-2);
    white-space: nowrap;
  }
  .ranges button.on {
    background: var(--bg-elev);
    color: var(--text);
    box-shadow: var(--shadow);
  }
  .totals {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
    margin: 0 0 14px;
  }
  .stat {
    background: var(--bg-column);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 8px 12px;
    color: var(--text-2);
    font-size: 12px;
  }
  .stat b {
    display: block;
    font-size: 20px;
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .stat.fin b {
    color: var(--s-done);
  }
  .stat.zero b {
    color: var(--text-3);
  }
  .empty {
    color: var(--text-3);
    text-align: center;
    padding: 40px 0;
  }
  .empty.err {
    color: var(--s-needs);
  }
  .group {
    margin-bottom: 12px;
  }
  h2,
  summary {
    display: flex;
    align-items: center;
    gap: 7px;
    margin: 0;
    padding: 4px;
    font-weight: 600;
    font-size: 12px;
    color: var(--text-2);
  }
  summary {
    cursor: default;
    list-style: none;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  summary::before {
    content: "›";
    color: var(--text-3);
    width: 8px;
    transition: transform 0.12s;
  }
  details[open] summary::before {
    transform: rotate(90deg);
  }
  .gdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--s-running);
  }
  .gcount {
    color: var(--text-3);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
  }
  .rows {
    display: flex;
    flex-direction: column;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    min-height: 48px;
    border: 0;
    border-top: 1px solid var(--border);
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    width: 100%;
  }
  .row:first-child {
    border-top: 0;
  }
  .row:hover {
    background: var(--bg-card-hover);
  }
  .row:focus-visible {
    outline: none;
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    gap: 10px;
    font-size: 11.5px;
    color: var(--text-3);
    margin-top: 1px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .model {
    font-size: 10.5px;
    padding: 0 5px;
    border-radius: 4px;
    background: var(--bg-hover);
    color: var(--text-2);
  }
  .when {
    font-variant-numeric: tabular-nums;
  }
  .text {
    font-size: 12px;
    color: var(--text-2);
    margin-top: 3px;
    white-space: pre-wrap;
  }
  .side {
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 1px;
    font-size: 12px;
    color: var(--text-2);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .work b {
    color: var(--text);
    font-weight: 600;
  }
  .ended {
    font-size: 11.5px;
  }
  .t-running {
    color: var(--s-running);
  }
  .t-needs {
    color: var(--s-needs);
    font-weight: 500;
  }
  .t-done {
    color: var(--s-done);
  }
  .t-idle {
    color: var(--s-idle);
  }
  .t-exited {
    color: var(--text-3);
  }
  @media (max-width: 560px) {
    .sum {
      padding: 4px 10px 24px;
    }
    .totals {
      grid-template-columns: repeat(2, 1fr);
    }
    /* The numbers go under the title instead of squeezing it. */
    .row {
      flex-wrap: wrap;
      padding: 8px 10px;
    }
    .main {
      flex-basis: calc(100% - 30px);
    }
    .meta {
      flex-wrap: wrap;
      row-gap: 0;
      white-space: normal;
    }
    .side {
      flex-direction: row;
      gap: 10px;
      margin-left: 30px;
    }
  }
</style>
