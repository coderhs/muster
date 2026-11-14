<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { placeBelow, portal } from "./actions";

  let {
    anchor,
    onclose,
    children,
    width = 200,
  }: { anchor: HTMLElement; onclose: () => void; children: Snippet; width?: number } = $props();

  let el: HTMLDivElement;
  let pos = $state({ x: 0, y: 0 });

  function place() {
    const p = placeBelow(anchor.getBoundingClientRect(), width, el?.offsetHeight ?? 0);
    pos = { x: p.x, y: p.y };
  }

  onMount(() => {
    place();
    const items = () => [...el.querySelectorAll<HTMLElement>("button:not(:disabled), input")];
    items()[0]?.focus();
    const down = (e: PointerEvent) => {
      if (!el.contains(e.target as Node) && !anchor.contains(e.target as Node)) onclose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onclose();
        anchor.focus();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        const list = items().filter((x) => x.tagName === "BUTTON");
        if (!list.length) return;
        e.preventDefault();
        e.stopPropagation();
        const i = list.indexOf(document.activeElement as HTMLElement);
        const n = e.key === "ArrowDown" ? (i + 1) % list.length : (i - 1 + list.length) % list.length;
        list[n].focus();
      }
    };
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("resize", onclose);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("resize", onclose);
    };
  });
</script>

<div class="pop" use:portal bind:this={el} style="left:{pos.x}px; top:{pos.y}px; width:{width}px" role="menu">
  {@render children()}
</div>

<style>
  .pop {
    position: fixed;
    z-index: 100;
    background: var(--bg-elev);
    border-radius: 8px;
    box-shadow: var(--shadow-pop);
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    animation: pop-in 0.08s ease-out;
  }
  @keyframes pop-in {
    from {
      opacity: 0;
      transform: translateY(-2px);
    }
  }
  .pop :global(.mi) {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    border: 0;
    background: transparent;
    text-align: left;
    padding: 4px 8px;
    border-radius: 5px;
    height: 24px;
    outline: none;
  }
  .pop :global(.mi:hover),
  .pop :global(.mi:focus-visible) {
    background: var(--accent);
    color: var(--accent-text);
  }
  .pop :global(.mi.danger) {
    color: var(--danger);
  }
  .pop :global(.mi.danger:hover),
  .pop :global(.mi.danger:focus-visible) {
    background: var(--danger);
    color: #fff;
  }
  .pop :global(.mi:disabled) {
    opacity: 0.4;
    background: transparent;
    color: inherit;
  }
  .pop :global(.mi .hint) {
    margin-left: auto;
    opacity: 0.6;
    font-size: 11px;
  }
  .pop :global(.sep) {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
  .pop :global(.sub) {
    padding: 4px 8px 2px;
    font-size: 11px;
    color: var(--text-3);
  }
</style>
