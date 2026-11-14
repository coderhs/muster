<script lang="ts" generics="T extends string">
  import { tick } from "svelte";
  import { placeBelow, portal } from "./actions";

  interface Option {
    value: T;
    label: string;
    color?: string;
    disabled?: boolean;
  }

  let {
    value,
    options,
    onchange,
    label,
    width,
    compact = false,
  }: {
    value: T;
    options: Option[];
    onchange: (v: T) => void;
    label: string;
    width?: string;
    compact?: boolean;
  } = $props();

  const uid = `sel-${Math.random().toString(36).slice(2, 8)}`;
  let btn = $state<HTMLButtonElement>();
  let list = $state<HTMLDivElement>();
  let open = $state(false);
  let active = $state(0);
  let pos = $state({ x: 0, y: 0, w: 0, maxHeight: 300 });
  let typed = "";
  let typedAt = 0;

  const current = $derived(options.find((o) => o.value === value));

  async function show() {
    if (open || !btn) return;
    active = Math.max(0, options.findIndex((o) => o.value === value));
    open = true;
    await tick();
    place();
    list?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function place() {
    if (!btn || !list) return;
    const r = btn.getBoundingClientRect();
    const w = Math.max(r.width, 140);
    const p = placeBelow(r, w, list.scrollHeight);
    pos = { x: p.x, y: p.y, w, maxHeight: p.maxHeight };
  }

  function hide(refocus = true) {
    open = false;
    if (refocus) btn?.focus();
  }

  function pick(i: number) {
    const o = options[i];
    if (!o || o.disabled) return;
    hide();
    if (o.value !== value) onchange(o.value);
  }

  function move(delta: number) {
    if (!options.length) return;
    let i = active;
    for (let n = 0; n < options.length; n++) {
      i = (i + delta + options.length) % options.length;
      if (!options[i].disabled) break;
    }
    active = i;
    list?.querySelector<HTMLElement>(`[data-i="${i}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function typeahead(ch: string) {
    const now = Date.now();
    typed = now - typedAt > 600 ? ch : typed + ch;
    typedAt = now;
    const start = open ? active : Math.max(0, options.findIndex((o) => o.value === value));
    const n = options.length;
    // A repeated single letter cycles; a longer prefix may match the current option.
    const from = typed.length > 1 ? start : start + 1;
    for (let k = 0; k < n; k++) {
      const j = (from + k) % n;
      const o = options[j];
      if (!o.disabled && o.label.toLowerCase().startsWith(typed.toLowerCase())) {
        if (open) {
          active = j;
          list?.querySelector<HTMLElement>(`[data-i="${j}"]`)?.scrollIntoView({ block: "nearest" });
        } else if (o.value !== value) onchange(o.value);
        return;
      }
    }
  }

  function onKey(e: KeyboardEvent) {
    const k = e.key;
    if (!open) {
      if (k === "ArrowDown" || k === "ArrowUp" || k === "Enter" || k === " ") {
        e.preventDefault();
        void show();
      } else if (k.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        typeahead(k);
      }
      return;
    }
    // While open every key belongs to the list; nothing reaches dialogs behind it.
    if (k === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      hide();
    } else if (k === "ArrowDown" || k === "ArrowUp") {
      e.preventDefault();
      move(k === "ArrowDown" ? 1 : -1);
    } else if (k === "Home" || k === "End") {
      e.preventDefault();
      active = k === "Home" ? 0 : options.length - 1;
    } else if (k === "Enter" || k === " ") {
      e.preventDefault();
      pick(active);
    } else if (k === "Tab") {
      hide(false);
    } else if (k.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      typeahead(k);
    }
  }

  $effect(() => {
    if (!open) return;
    const down = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!list?.contains(t) && !btn?.contains(t)) hide(false);
    };
    const close = () => hide(false);
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
    };
  });
</script>

<button
  bind:this={btn}
  type="button"
  class="sel field"
  class:compact
  style:width
  role="combobox"
  aria-label={label}
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-controls={uid}
  aria-activedescendant={open ? `${uid}-${active}` : undefined}
  onclick={() => (open ? hide() : void show())}
  onkeydown={onKey}
>
  {#if current?.color}<span class="dot" style="background:{current.color}"></span>{/if}
  <span class="txt">{current?.label ?? ""}</span>
  <svg class="chev" width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"
    ><path d="M2.5 4l2.5 2.5L7.5 4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg
  >
</button>

{#if open}
  <div
    use:portal
    bind:this={list}
    id={uid}
    class="lb"
    role="listbox"
    aria-label={label}
    style="left:{pos.x}px; top:{pos.y}px; min-width:{pos.w}px; max-height:{pos.maxHeight}px"
  >
    {#each options as o, i (o.value)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        id="{uid}-{i}"
        data-i={i}
        class="opt"
        class:active={i === active}
        class:selected={o.value === value}
        class:disabled={o.disabled}
        role="option"
        tabindex="-1"
        aria-selected={o.value === value}
        aria-disabled={o.disabled}
        onpointermove={() => !o.disabled && (active = i)}
        onpointerdown={(e) => e.preventDefault()}
        onclick={() => pick(i)}
      >
        <span class="check">{#if o.value === value}✓{/if}</span>
        {#if o.color}<span class="dot" style="background:{o.color}"></span>{/if}
        <span class="txt">{o.label}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .sel {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    text-align: left;
    min-width: 0;
    cursor: default;
  }
  .sel.compact {
    width: auto;
    padding: 2px 6px;
  }
  .txt {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chev {
    flex: none;
    color: var(--text-2);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .lb {
    position: fixed;
    z-index: 400;
    overflow-y: auto;
    padding: 4px;
    background: var(--bg-elev);
    border-radius: 8px;
    box-shadow: var(--shadow-pop);
    font: 13px/1.4 var(--font);
    color: var(--text);
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 10px 0 4px;
    border-radius: 5px;
    white-space: nowrap;
  }
  .opt.active {
    background: var(--accent);
    color: var(--accent-text);
  }
  .opt.disabled {
    opacity: 0.4;
  }
  .check {
    width: 14px;
    text-align: center;
    font-size: 11px;
  }
</style>
