<script lang="ts">
  import type { Agent } from "../protocol/types";
  import { useApp } from "../state/context";
  import { clock } from "../state/clock.svelte";
  import { level } from "../agent/agent";
  import { limitRows, modelsInUse, resetText, signIn, signInLabel, worstRow } from "../agent/usage";
  import { agoLabel } from "./time";
  import AgentMark from "./AgentMark.svelte";
  import Icon from "./Icon.svelte";

  const app = useApp();

  // Ask once per connection; the daemon caches the answer for a minute.
  $effect(() => {
    void app.epoch;
    app.loadAccounts();
  });

  const OPEN_KEY = "muster.tracker.agents";
  let open = $state(recall());
  function recall() {
    try {
      return localStorage.getItem(OPEN_KEY) !== "0";
    } catch {
      return true;
    }
  }
  function toggle() {
    open = !open;
    try {
      localStorage.setItem(OPEN_KEY, open ? "1" : "0");
    } catch {
      /* ignore */
    }
  }

  const cards = $derived(Object.values(app.allCards));
  /** Agents worth a line: installed, signed in, or with limits on record. */
  const entries = $derived(
    app.agents
      .filter((a) => a.id !== "shell")
      .map((agent) => {
        const account = app.accounts?.find((x) => x.agentId === agent.id) ?? null;
        const usage = app.usage[agent.id] ?? null;
        return {
          agent,
          account,
          usage,
          rows: limitRows(usage, agent.id, clock.now),
          models: modelsInUse(cards, agent.id),
        };
      })
      .filter((e) => e.rows.length || e.models.length || (e.account ? e.account.installed : e.agent.installed)),
  );
  const withLimits = $derived(entries.filter((e) => e.rows.length));
  const others = $derived(entries.filter((e) => !e.rows.length));
  /** Collapsed header: the fullest window of each agent that has limits. */
  const summary = $derived(
    withLimits
      .map((e) => ({ agent: e.agent, row: worstRow(e.rows) }))
      .filter((s): s is { agent: Agent; row: NonNullable<typeof s.row> } => !!s.row),
  );
  const pctText = (p: number) => `${Math.round(p)}%`;
</script>

{#if entries.length}
  <section class="agents">
    <div class="head">
      <button class="toggle" onclick={toggle} aria-expanded={open}>
        <span class="chev" class:open><Icon name="chevronRight" size={12} /></span>
        Agents and limits
      </button>
      {#if !open}
        <span class="sum">
          {#each summary as s (s.agent.id)}
            <span class="sumi l-{level(s.row.usedPercent, 75, 90)}" title="{s.agent.label}: {s.row.label}">
              {s.agent.label} <b>{pctText(s.row.usedPercent)}</b>
            </span>
          {/each}
        </span>
      {/if}
      {#if open}
        <button class="again" title="Check sign-in again" aria-label="Check sign-in again" onclick={() => app.loadAccounts(true)}>
          <Icon name="restore" size={12} />
        </button>
      {/if}
    </div>

    {#if open}
      {#each withLimits as e (e.agent.id)}
        <div class="block">
          <div class="who">
            <AgentMark agent={e.agent} size={18} />
            <span class="name">{e.agent.label}</span>
            <span class="pill s-{signIn(e.account)}">{signInLabel(e.account)}</span>
            {#if e.usage}
              <span class="ago" title="Limits as of the last report from a {e.agent.label} session in Muster">
                as of {agoLabel(e.usage.updatedAt, clock.now)}
              </span>
            {/if}
          </div>
          {#each e.rows as r (r.key)}
            <div class="lim l-{level(r.usedPercent, 75, 90)}">
              <span class="lbl">{r.label}</span>
              <span class="bar"><span class="fill" style="width:{r.usedPercent}%"></span></span>
              <span class="pct">{pctText(r.usedPercent)} used</span>
              {#if resetText(r, clock.now)}<span class="reset">{resetText(r, clock.now)}</span>{/if}
            </div>
          {/each}
          {@render extras(e)}
        </div>
      {/each}
      {#if others.length}
        <div class="list">
          {#each others as e (e.agent.id)}
            <div class="line">
              <AgentMark agent={e.agent} size={16} />
              <span class="name">{e.agent.label}</span>
              <span class="pill s-{signIn(e.account)}">{signInLabel(e.account)}</span>
              {#if e.account?.detail}<span class="muted">{e.account.detail}</span>{/if}
              {@render extras(e)}
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </section>
{/if}

{#snippet extras(e: (typeof entries)[number])}
  {#if e.models.length || e.account?.defaultModel || (e.account?.usageNote && e.account.installed)}
    <div class="extra">
      {#if e.models.length}
        <span class="models" title="Models of live sessions on the board">
          {#each e.models as m (m.model)}
            <span class="model" class:busy={m.running > 0}>{m.model}{m.live > 1 ? ` ×${m.live}` : ""}</span>
          {/each}
        </span>
      {/if}
      {#if e.account?.defaultModel}<span class="muted">Default: {e.account.defaultModel}</span>{/if}
      {#if e.account?.usageNote && e.account.installed}<span class="note">{e.account.usageNote}</span>{/if}
    </div>
  {/if}
{/snippet}

<style>
  .agents {
    background: var(--bg-column);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 6px 12px 8px;
    margin: 4px 0 10px;
    font-size: 12px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-weight: 600;
    color: var(--text-2);
    background: none;
    border: 0;
    padding: 2px 0;
    flex: none;
  }
  .chev {
    display: inline-flex;
    color: var(--text-3);
    transition: transform 0.15s;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .sum {
    display: flex;
    gap: 10px;
    overflow: hidden;
    white-space: nowrap;
    color: var(--text-3);
  }
  .sumi b {
    color: var(--c);
    font-variant-numeric: tabular-nums;
  }
  .again {
    margin-left: auto;
    color: var(--text-3);
    background: none;
    border: 0;
    display: inline-flex;
    padding: 3px;
    border-radius: 5px;
  }
  .again:hover {
    color: var(--text);
    background: var(--bg-active);
  }
  .block {
    border-top: 1px solid var(--border);
    margin-top: 6px;
    padding-top: 7px;
  }
  .who,
  .line {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .name {
    font-weight: 600;
    color: var(--text);
  }
  .pill {
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--bg-active);
    color: var(--text-3);
    white-space: nowrap;
  }
  .pill.s-signed_in {
    color: var(--s-done);
    background: color-mix(in srgb, var(--s-done) 14%, transparent);
  }
  .pill.s-signed_out {
    color: var(--s-needs);
    background: color-mix(in srgb, var(--s-needs) 14%, transparent);
  }
  .ago {
    margin-left: auto;
    color: var(--text-3);
    font-size: 11px;
  }
  .lim {
    --c: var(--s-done);
    display: grid;
    grid-template-columns: minmax(150px, 190px) minmax(60px, 1fr) auto;
    grid-template-areas: "lbl bar pct" "reset reset reset";
    align-items: center;
    column-gap: 10px;
    margin-top: 6px;
  }
  .lbl {
    grid-area: lbl;
    color: var(--text-2);
  }
  .bar {
    grid-area: bar;
    height: 6px;
    border-radius: 3px;
    background: var(--bg-active);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--c);
    border-radius: 3px;
    transition: width 0.3s;
  }
  .pct {
    grid-area: pct;
    font-weight: 600;
    color: var(--c);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .reset {
    grid-area: reset;
    color: var(--text-3);
    font-size: 11px;
  }
  .l-warn {
    --c: var(--s-needs);
  }
  .l-crit {
    --c: var(--danger);
  }
  .list {
    border-top: 1px solid var(--border);
    margin-top: 7px;
    padding-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .line {
    padding-top: 3px;
  }
  .extra {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
    margin-top: 5px;
    width: 100%;
    padding-left: 25px;
  }
  .line .extra {
    width: auto;
    margin-top: 0;
    padding-left: 0;
  }
  .models {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .model {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 5px;
    border: 1px solid var(--border);
    color: var(--text-2);
    white-space: nowrap;
  }
  .model.busy {
    border-color: color-mix(in srgb, var(--s-running) 50%, var(--border));
  }
  .muted {
    color: var(--text-3);
  }
  .note {
    color: var(--text-3);
    font-size: 11px;
    flex-basis: 100%;
  }
  .line .note {
    flex-basis: auto;
  }
  @media (max-width: 560px) {
    .lim {
      grid-template-columns: 1fr auto;
      grid-template-areas: "lbl pct" "bar bar" "reset reset";
      row-gap: 3px;
    }
    .ago {
      margin-left: 0;
    }
  }
</style>
