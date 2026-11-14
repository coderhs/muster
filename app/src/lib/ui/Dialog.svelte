<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  let {
    onclose,
    children,
    width = 480,
    label,
    closeOnEscape = true,
  }: { onclose: () => void; children: Snippet; width?: number | string; label: string; closeOnEscape?: boolean } =
    $props();

  let box: HTMLDivElement;
  let restore: HTMLElement | null = null;

  onMount(() => {
    restore = document.activeElement as HTMLElement | null;
    const first =
      box.querySelector<HTMLElement>("[data-autofocus]") ??
      box.querySelector<HTMLElement>("input, textarea, select, button");
    first?.focus();
    return () => restore?.focus?.();
  });

  function key(e: KeyboardEvent) {
    if (e.key === "Escape" && closeOnEscape && !e.defaultPrevented) {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
    if (e.key === "Tab") {
      // Keep focus inside the dialog.
      const f = [...box.querySelectorAll<HTMLElement>("input, textarea, select, button, [tabindex]")].filter(
        (x) => !x.hasAttribute("disabled") && x.tabIndex >= 0 && x.offsetParent !== null,
      );
      if (!f.length) return;
      const i = f.indexOf(document.activeElement as HTMLElement);
      if (e.shiftKey && i <= 0) {
        e.preventDefault();
        f[f.length - 1].focus();
      } else if (!e.shiftKey && i === f.length - 1) {
        e.preventDefault();
        f[0].focus();
      }
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="scrim"
  onpointerdown={(e) => {
    if (e.target === e.currentTarget) onclose();
  }}
  onkeydown={key}
>
  <div
    class="dialog"
    bind:this={box}
    role="dialog"
    aria-modal="true"
    aria-label={label}
    style="width:{typeof width === 'number' ? `${width}px` : width}"
  >
    {@render children()}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: var(--scrim);
    display: grid;
    place-items: center;
    animation: fade 0.1s ease-out;
    padding: 24px;
  }
  .dialog {
    max-width: 100%;
    max-height: 100%;
    background: var(--bg-elev);
    border-radius: 12px;
    box-shadow: var(--shadow-pop);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: rise 0.12s ease-out;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      transform: scale(0.98);
      opacity: 0;
    }
  }
</style>
