<script lang="ts">
  // A changed image in the Changes tab: before and after, side by side.
  import type { CardImage, ImageSide } from "../protocol/types";
  import { bytesLabel, sizeChange } from "../changes/image";

  let { image }: { image: CardImage } = $props();

  type Side = { key: "old" | "new"; label: string; side: ImageSide };
  const sides = $derived(
    [
      image.old && { key: "old" as const, label: image.new ? "Before" : "Removed", side: image.old },
      image.new && { key: "new" as const, label: image.old ? "After" : "Added", side: image.new },
    ].filter((s): s is Side => !!s),
  );
  // Natural size of each picture once it has loaded, or "failed". The parent remounts
  // this component for another image, so this starts empty for each one.
  let dims = $state<Record<string, { w: number; h: number } | "failed">>({});
  const change = $derived(sizeChange(image.old?.bytes, image.new?.bytes));

  function loaded(key: string, e: Event) {
    const img = e.currentTarget as HTMLImageElement;
    dims = { ...dims, [key]: { w: img.naturalWidth, h: img.naturalHeight } };
  }
</script>

<div class="images">
  {#if !sides.length}
    <p class="none">No picture to show: the file is a symbolic link or could not be read.</p>
  {/if}
  {#each sides as s (s.key)}
    {@const d = dims[s.key]}
    <figure class="side {s.key}">
      <figcaption>
        <b>{s.label}</b>
        <span>
          {#if d && d !== "failed"}{d.w} × {d.h} ·{/if}
          {bytesLabel(s.side.bytes)}{#if s.key === "new" && change}<span class="delta">{change}</span>{/if}
        </span>
      </figcaption>
      <div class="frame">
        {#if !s.side.data}
          <p class="none">Too large to show ({bytesLabel(s.side.bytes)}).</p>
        {:else if d === "failed"}
          <p class="none">This file cannot be displayed.</p>
        {:else}
          <img
            src={`data:${image.mime};base64,${s.side.data}`}
            alt={`${s.label}: ${image.path}`}
            onload={(e) => loaded(s.key, e)}
            onerror={() => (dims = { ...dims, [s.key]: "failed" })}
          />
        {/if}
      </div>
    </figure>
  {/each}
</div>

<style>
  .images {
    display: flex;
    gap: 12px;
    padding: 12px;
    align-items: flex-start;
  }
  .side {
    flex: 1;
    min-width: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  figcaption {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 12px;
    color: var(--text-2);
  }
  figcaption b {
    font-weight: 600;
  }
  .old figcaption b {
    color: var(--danger);
  }
  .new figcaption b {
    color: var(--s-done);
  }
  .delta {
    margin-left: 6px;
    color: var(--text-3);
  }
  .frame {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 120px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    /* A checkerboard, so transparent parts of the picture show as such. */
    background-color: var(--bg-elev);
    background-image:
      linear-gradient(45deg, var(--bg-hover) 25%, transparent 25%, transparent 75%, var(--bg-hover) 75%),
      linear-gradient(45deg, var(--bg-hover) 25%, transparent 25%, transparent 75%, var(--bg-hover) 75%);
    background-size: 16px 16px;
    background-position:
      0 0,
      8px 8px;
  }
  .old .frame {
    border-color: color-mix(in srgb, var(--danger) 35%, transparent);
  }
  .new .frame {
    border-color: color-mix(in srgb, var(--s-done) 35%, transparent);
  }
  img {
    max-width: 100%;
    max-height: 60vh;
    object-fit: contain;
    image-rendering: auto;
  }
  .none {
    margin: 0;
    font-size: 12px;
    color: var(--text-2);
  }
  .images > .none {
    padding: 0 0 6px;
  }
</style>
