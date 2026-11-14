<script lang="ts">
  import type { Card } from "../protocol/types";
  import { useApp } from "../state/context";
  import { clock } from "../state/clock.svelte";
  import { shortDuration } from "./time";
  import Icon from "./Icon.svelte";

  let { card, compact = false }: { card: Card; compact?: boolean } = $props();
  const app = useApp();

  const a = $derived(card.pendingApproval!);
  const kind = $derived(a?.kind ?? "tool");
  const viaHook = $derived(!!a?.requestId);
  let busy = $state(false);
  let remember = $state(false);
  let message = $state("");
  let sendingBack = $state(false);
  /** Chosen labels per question text, and free text per question. */
  let chosen = $state<Record<string, string[]>>({});
  let other = $state<Record<string, string>>({});

  const questions = $derived(a?.questions ?? []);
  const oneTap = $derived(questions.length === 1 && !questions[0].multiSelect && questions[0].options.length <= 4);
  const complete = $derived(questions.every((q) => (chosen[q.question]?.length ?? 0) > 0 || (other[q.question] ?? "").trim()));

  async function run(f: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      await f();
    } catch {
      /* toast shown */
    } finally {
      busy = false;
    }
  }

  const answerTool = (decision: "allow" | "deny") =>
    run(() => app.answerApproval(card, decision, viaHook ? { remember, message: decision === "deny" ? message.trim() : "" } : {}));

  function toggle(q: string, label: string, multi: boolean) {
    const cur = chosen[q] ?? [];
    chosen = { ...chosen, [q]: multi ? (cur.includes(label) ? cur.filter((x) => x !== label) : [...cur, label]) : [label] };
    if (!multi) other = { ...other, [q]: "" };
  }

  function submitAnswers() {
    const answers: Record<string, string[]> = {};
    for (const q of questions) {
      const free = (other[q.question] ?? "").trim();
      answers[q.question] = free ? [...(q.multiSelect ? (chosen[q.question] ?? []) : []), free] : (chosen[q.question] ?? []);
    }
    return run(() => app.answerPrompt(card, { decision: "answer", answers }));
  }

  const stop = (e: Event) => e.stopPropagation();
</script>

{#if a}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="appr k-{kind}" class:compact onclick={stop} onpointerdown={stop} ondblclick={stop}>
    {#if kind === "question"}
      {#if compact}
        <div class="what"><Icon name="warning" size={12} /><span class="summary">{questions[0]?.header || questions[0]?.question || a.summary}</span></div>
        <div class="btns">
          {#if oneTap && viaHook}
            {#each questions[0].options as o (o.label)}
              <button class="btn small" disabled={busy} title={o.description ?? ""} onclick={() => run(() => app.answerPrompt(card, { decision: "answer", answers: { [questions[0].question]: [o.label] } }))}>{o.label}</button>
            {/each}
          {:else}
            <button class="btn small primary" onclick={() => app.openCard(card.id)}>Answer…</button>
          {/if}
        </div>
      {:else}
        <div class="what"><Icon name="warning" size={14} /><span class="lead">{questions.length === 1 ? "A question for you" : `${questions.length} questions for you`}</span><span class="since">waiting {shortDuration(clock.now - a.since)}</span></div>
        {#each questions as q (q.question)}
          <fieldset class="q">
            {#if q.header}<span class="hdr">{q.header}</span>{/if}
            <legend>{q.question}</legend>
            <div class="opts-list" role={q.multiSelect ? "group" : "radiogroup"} aria-label={q.question}>
              {#each q.options as o (o.label)}
                <button
                  class="opt"
                  role={q.multiSelect ? "checkbox" : "radio"}
                  aria-checked={(chosen[q.question] ?? []).includes(o.label)}
                  class:on={(chosen[q.question] ?? []).includes(o.label)}
                  disabled={!viaHook || busy}
                  onclick={() => toggle(q.question, o.label, q.multiSelect)}
                >
                  <span class="mark">{q.multiSelect ? ((chosen[q.question] ?? []).includes(o.label) ? "☑" : "☐") : (chosen[q.question] ?? []).includes(o.label) ? "◉" : "○"}</span>
                  <span class="ol"><b>{o.label}</b>{#if o.description}<small>{o.description}</small>{/if}</span>
                </button>
              {/each}
              <input
                class="field other"
                placeholder="Other answer…"
                disabled={!viaHook || busy}
                value={other[q.question] ?? ""}
                oninput={(e) => {
                  other = { ...other, [q.question]: (e.currentTarget as HTMLInputElement).value };
                  if (!q.multiSelect && (e.currentTarget as HTMLInputElement).value) chosen = { ...chosen, [q.question]: [] };
                }}
              />
            </div>
          </fieldset>
        {/each}
        {#if !viaHook}<div class="note">This question can only be answered in the terminal now (the hook request is gone).</div>{/if}
        <div class="btns">
          <button class="btn small" disabled={busy || !viaHook} onclick={() => run(() => app.answerPrompt(card, { decision: "deny", message: "The user declined to answer." }))}>Decline</button>
          <button class="btn small primary" disabled={busy || !viaHook || !complete} onclick={submitAnswers}>Send answer{questions.length > 1 ? "s" : ""}</button>
        </div>
      {/if}
    {:else if kind === "plan"}
      {#if compact}
        <div class="what"><Icon name="check" size={12} /><span class="summary">Plan ready for review</span></div>
        <div class="btns">
          <button class="btn small" onclick={() => app.openCard(card.id)}>Review</button>
          <button class="btn small primary" disabled={busy || !viaHook} onclick={() => run(() => app.answerPrompt(card, { decision: "approve_plan" }))}>Approve</button>
        </div>
      {:else}
        <div class="what"><Icon name="check" size={14} /><span class="lead">Plan ready for review</span><span class="since">waiting {shortDuration(clock.now - a.since)}</span></div>
        {#if a.plan}<pre class="plan">{a.plan}</pre>{/if}
        {#if a.planFilePath}<div class="note">Saved at <code>{a.planFilePath}</code></div>{/if}
        {#if !viaHook}<div class="note">This plan can only be answered in the terminal now (the hook request is gone).</div>{/if}
        {#if sendingBack}
          <input class="field msg" placeholder="What should change? (sent to the agent)" bind:value={message} />
        {/if}
        <div class="btns">
          {#if sendingBack}
            <button class="btn small" onclick={() => (sendingBack = false)}>Cancel</button>
            <button class="btn small primary" disabled={busy || !message.trim()} onclick={() => run(() => app.answerPrompt(card, { decision: "deny", message: message.trim() }))}>Send back</button>
          {:else}
            <button class="btn small" disabled={busy || !viaHook} onclick={() => (sendingBack = true)}>Send back…</button>
            <button class="btn small" disabled={busy || !viaHook} title="Approve and let the agent edit files without asking" onclick={() => run(() => app.answerPrompt(card, { decision: "approve_plan", acceptEdits: true }))}>Approve, accept edits</button>
            <button class="btn small primary" disabled={busy || !viaHook} onclick={() => run(() => app.answerPrompt(card, { decision: "approve_plan" }))}>Approve</button>
          {/if}
        </div>
      {/if}
    {:else}
      <div class="what">
        <Icon name="warning" size={compact ? 12 : 14} />
        <span class="summary" title={a.summary}>{a.summary || a.tool}</span>
        {#if !compact}<span class="since">waiting {shortDuration(clock.now - a.since)}</span>{/if}
      </div>
      {#if !compact && viaHook}
        <div class="opts">
          <label><input type="checkbox" bind:checked={remember} /> Always allow this</label>
          <input class="field msg" placeholder="Reason, if you deny (shown to the agent)" bind:value={message} />
        </div>
      {:else if !compact}
        <div class="note">Answered by typing into the terminal: Allow sends 1, Deny sends Escape (which also interrupts the turn).</div>
      {/if}
      <div class="btns">
        <button class="btn small deny" disabled={busy} onclick={() => answerTool("deny")}>Deny</button>
        <button class="btn small primary allow" disabled={busy} onclick={() => answerTool("allow")}>
          <Icon name="check" size={11} />Allow
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .appr {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--s-needs) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--s-needs) 35%, transparent);
    cursor: default;
  }
  .appr.k-plan {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .appr.compact {
    flex-direction: row;
    align-items: center;
    gap: 6px;
    padding: 4px 4px 4px 8px;
  }
  .what {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1;
    color: var(--s-needs);
  }
  .k-plan .what {
    color: var(--accent);
  }
  .summary {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .k-question .summary,
  .k-plan .summary {
    font-family: var(--font);
  }
  .compact .summary {
    font-size: 11px;
  }
  .lead {
    color: var(--text);
    font-weight: 600;
  }
  .since {
    margin-left: auto;
    font-size: 11px;
    color: var(--text-2);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .opts {
    display: flex;
    gap: 10px;
    align-items: center;
    flex-wrap: wrap;
    font-size: 12px;
    color: var(--text-2);
  }
  .opts label {
    display: inline-flex;
    gap: 5px;
    align-items: center;
    white-space: nowrap;
  }
  .msg {
    flex: 1;
    min-width: 160px;
    height: 26px;
    font-size: 12px;
  }
  .note {
    font-size: 11.5px;
    color: var(--text-2);
  }
  .note code {
    font: 11px var(--mono);
    -webkit-user-select: text;
    user-select: text;
  }
  .q {
    border: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .hdr {
    align-self: flex-start;
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-2);
  }
  legend {
    float: left;
    font-weight: 500;
    padding: 0;
  }
  .opts-list {
    clear: both;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .opt {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    text-align: left;
    border: 1px solid var(--border-strong);
    background: var(--bg-elev);
    border-radius: 7px;
    padding: 6px 10px;
  }
  .opt:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .opt.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .opt:disabled {
    opacity: 0.6;
  }
  .mark {
    width: 14px;
    color: var(--accent);
  }
  .ol {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ol small {
    color: var(--text-2);
    font-size: 11.5px;
  }
  .other {
    height: 28px;
    font-size: 12px;
  }
  .plan {
    margin: 0;
    max-height: 260px;
    overflow: auto;
    white-space: pre-wrap;
    font: 12px/1.5 var(--font);
    background: var(--bg-elev);
    border-radius: 6px;
    padding: 8px 10px;
    -webkit-user-select: text;
    user-select: text;
  }
  .btns {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
    flex: none;
    flex-wrap: wrap;
  }
  .compact .btn {
    height: 20px;
    padding: 0 7px;
    font-size: 11px;
  }
  .deny {
    color: var(--danger);
  }
</style>
