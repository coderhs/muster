<script lang="ts">
  import { PALETTE } from "./colors";
  let { value, onpick }: { value: string; onpick: (c: string) => void } = $props();
</script>

<div class="sw">
  {#each PALETTE as c (c)}
    <button
      class:on={c.toLowerCase() === value?.toLowerCase()}
      style="--c:{c}"
      title={c}
      aria-label="Colour {c}"
      onclick={() => onpick(c)}
    ></button>
  {/each}
</div>

<style>
  .sw {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 6px;
    padding: 4px 8px 6px;
  }
  button {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 0;
    background: var(--c);
    padding: 0;
    box-shadow: inset 0 0 0 0.5px rgba(0, 0, 0, 0.25);
  }
  button.on {
    box-shadow:
      0 0 0 2px var(--bg-elev),
      0 0 0 4px var(--c);
  }
  button:focus-visible {
    outline: 2px solid var(--focus);
  }
</style>
