<script lang="ts">
  import { tick } from "svelte";
  import { placeBelow, portal } from "./actions";
  import Icon from "./Icon.svelte";

  /** Value is a local end-of-day timestamp (ms) or null. */
  let { value, onchange, label = "Date" }: { value: number | null; onchange: (v: number | null) => void; label?: string } =
    $props();

  let btn = $state<HTMLButtonElement>();
  let pop = $state<HTMLDivElement>();
  let open = $state(false);
  let pos = $state({ x: 0, y: 0 });
  // The day with keyboard focus inside the grid, and the month shown.
  let focusDay = $state(new Date());
  const view = $derived(new Date(focusDay.getFullYear(), focusDay.getMonth(), 1));

  const dayStart = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
  const endOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate(), 23, 59).getTime();
  const fmt = (t: number) => new Date(t).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric", year: "numeric" });

  // Weeks start on the locale's first day where the browser says so; Monday otherwise.
  const firstDow = (() => {
    try {
      const info = (new Intl.Locale(navigator.language) as unknown as { weekInfo?: { firstDay: number } }).weekInfo;
      return info ? info.firstDay % 7 : 1;
    } catch {
      return 1;
    }
  })();
  const dowNames = $derived(
    Array.from({ length: 7 }, (_, i) =>
      new Date(2024, 0, 7 + ((firstDow + i) % 7)).toLocaleDateString(undefined, { weekday: "narrow" }),
    ),
  );
  const days = $derived.by(() => {
    const start = new Date(view);
    start.setDate(1 - ((view.getDay() - firstDow + 7) % 7));
    return Array.from({ length: 42 }, (_, i) => new Date(start.getFullYear(), start.getMonth(), start.getDate() + i));
  });

  async function show() {
    focusDay = value ? dayStart(new Date(value)) : dayStart(new Date());
    open = true;
    await tick();
    if (btn && pop) {
      const p = placeBelow(btn.getBoundingClientRect(), pop.offsetWidth, pop.offsetHeight);
      pos = { x: p.x, y: p.y };
    }
    pop?.querySelector<HTMLElement>(".day.focus")?.focus();
  }

  function hide() {
    open = false;
    btn?.focus();
  }

  function pick(d: Date | null) {
    open = false;
    btn?.focus();
    onchange(d ? endOfDay(d) : null);
  }

  async function shift(days: number, months = 0) {
    const d = new Date(focusDay);
    if (months) d.setMonth(d.getMonth() + months);
    d.setDate(d.getDate() + days);
    focusDay = d;
    await tick();
    pop?.querySelector<HTMLElement>(".day.focus")?.focus();
  }

  function onKey(e: KeyboardEvent) {
    const map: Record<string, () => void> = {
      ArrowLeft: () => shift(-1),
      ArrowRight: () => shift(1),
      ArrowUp: () => shift(-7),
      ArrowDown: () => shift(7),
      PageUp: () => shift(0, -1),
      PageDown: () => shift(0, 1),
      Enter: () => pick(focusDay),
      " ": () => pick(focusDay),
      Escape: () => hide(),
      t: () => {
        focusDay = dayStart(new Date());
        void shift(0);
      },
    };
    const f = map[e.key];
    if (f) {
      e.preventDefault();
      e.stopPropagation();
      f();
    }
  }

  $effect(() => {
    if (!open) return;
    const down = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!pop?.contains(t) && !btn?.contains(t)) open = false;
    };
    const close = () => (open = false);
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("resize", close);
    };
  });

  const today = dayStart(new Date());
</script>

<button
  bind:this={btn}
  type="button"
  class="field dp"
  aria-label={label}
  aria-haspopup="dialog"
  aria-expanded={open}
  onclick={() => (open ? hide() : void show())}
  onkeydown={(e) => {
    if (e.key === "ArrowDown" && !open) {
      e.preventDefault();
      void show();
    } else if ((e.key === "Backspace" || e.key === "Delete") && value) {
      e.preventDefault();
      onchange(null);
    }
  }}
>
  <Icon name="calendar" size={13} />
  <span class:none={!value}>{value ? fmt(value) : "No due date"}</span>
</button>

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div use:portal bind:this={pop} class="cal" role="dialog" aria-label={label} tabindex="-1" style="left:{pos.x}px; top:{pos.y}px" onkeydown={onKey}>
    <div class="head">
      <button type="button" class="icon-btn" aria-label="Previous month" onclick={() => shift(0, -1)}><Icon name="chevronLeft" size={14} /></button>
      <span class="month">{view.toLocaleDateString(undefined, { month: "long", year: "numeric" })}</span>
      <button type="button" class="icon-btn" aria-label="Next month" onclick={() => shift(0, 1)}><Icon name="chevronRight" size={14} /></button>
    </div>
    <div class="grid" role="grid">
      {#each dowNames as d, i (i)}<span class="dow" role="columnheader">{d}</span>{/each}
      {#each days as d (d.getTime())}
        <button
          type="button"
          role="gridcell"
          class="day"
          class:other={d.getMonth() !== view.getMonth()}
          class:today={sameDay(d, today)}
          class:sel={value != null && sameDay(d, new Date(value))}
          class:focus={sameDay(d, focusDay)}
          tabindex={sameDay(d, focusDay) ? 0 : -1}
          aria-selected={value != null && sameDay(d, new Date(value))}
          aria-label={d.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric", year: "numeric" })}
          onclick={() => pick(d)}>{d.getDate()}</button
        >
      {/each}
    </div>
    <div class="foot">
      <button type="button" class="btn small" onclick={() => pick(today)}>Today</button>
      <button type="button" class="btn small" onclick={() => pick(new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1))}>Tomorrow</button>
      <span class="spacer"></span>
      <button type="button" class="btn small" disabled={!value} onclick={() => pick(null)}>Clear</button>
    </div>
  </div>
{/if}

<style>
  .dp {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    text-align: left;
    color: var(--text);
  }
  .dp :global(svg) {
    color: var(--text-2);
    flex: none;
  }
  .none {
    color: var(--text-3);
  }
  .cal {
    position: fixed;
    z-index: 400;
    width: 248px;
    padding: 8px;
    background: var(--bg-elev);
    border-radius: 10px;
    box-shadow: var(--shadow-pop);
    font: 13px/1.4 var(--font);
    color: var(--text);
    outline: none;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 4px;
  }
  .month {
    font-weight: 600;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 2px;
  }
  .dow {
    text-align: center;
    font-size: 10.5px;
    color: var(--text-3);
    padding: 2px 0;
  }
  .day {
    height: 28px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    padding: 0;
  }
  .day:hover {
    background: var(--bg-hover);
  }
  .day.other {
    color: var(--text-3);
  }
  .day.today {
    color: var(--accent);
    font-weight: 700;
  }
  .day.sel {
    background: var(--accent);
    color: var(--accent-text);
  }
  .day.focus {
    outline: 2px solid var(--focus);
    outline-offset: -1px;
  }
  .foot {
    display: flex;
    gap: 6px;
    margin-top: 8px;
  }
  .spacer {
    flex: 1;
  }
</style>
