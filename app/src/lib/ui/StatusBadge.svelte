<script lang="ts" module>
  import type { CardStatus } from "../protocol/types";
  export const STATUS_LABEL: Record<CardStatus, string> = {
    draft: "Draft",
    running: "Running",
    needs_you: "Needs you",
    done: "Done",
    idle: "Idle",
    exited: "Exited",
  };
</script>

<script lang="ts">
  import { clock } from "../state/clock.svelte";
  import { shortDuration } from "./time";

  let {
    status,
    since,
    settingUp = false,
    source = "hooks",
  }: { status: CardStatus; since: number; settingUp?: boolean; source?: "hooks" | "log" | "activity" } = $props();
  const SOURCE_NOTE = {
    hooks: "",
    log: "Status from the agent's session log",
    activity: "Status from terminal activity: running while it prints, idle after a quiet spell",
  } as const;
  const showTimer = $derived(status !== "draft" && since > 0);
</script>

<span class="badge s-{status}" class:setup={settingUp} title={SOURCE_NOTE[source] || undefined}>
  <span class="dot"></span>
  {settingUp ? "Setting up" : (STATUS_LABEL[status] ?? status)}
  {#if showTimer}<span class="for">for {shortDuration(clock.now - since)}</span>{/if}
</span>

<style>
  .badge {
    --c: var(--s-idle);
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 18px;
    padding: 0 7px 0 6px;
    border-radius: 9px;
    font-size: 11px;
    font-weight: 500;
    color: var(--c);
    background: color-mix(in srgb, var(--c) 14%, transparent);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--c);
  }
  .for {
    color: var(--text-2);
    font-weight: 400;
  }
  .s-running {
    --c: var(--s-running);
  }
  .s-running .dot {
    animation: pulse 1.6s ease-in-out infinite;
  }
  .badge.setup {
    --c: #bf5af2;
  }
  .s-needs_you {
    --c: var(--s-needs);
  }
  .s-done {
    --c: var(--s-done);
  }
  .s-idle {
    --c: var(--s-idle);
  }
  .s-exited {
    --c: var(--s-exited);
  }
  .s-draft {
    --c: var(--s-draft);
  }
  .s-draft .dot {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--c);
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .s-running .dot {
      animation: none;
    }
  }
</style>
