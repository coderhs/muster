<script lang="ts">
  import Select from "./Select.svelte";
  import { focusOnMount } from "./actions";
  import type { Column, ColumnCategory } from "../protocol/types";
  import { useApp } from "../state/context";
  import Icon from "./Icon.svelte";
  import Popover from "./Popover.svelte";
  import Swatches from "./Swatches.svelte";
  import Dialog from "./Dialog.svelte";
  import { CATEGORY_COLOR, CATEGORY_LABEL, colorOr } from "./colors";

  let { column, count, index, total }: { column: Column; count: number; index: number; total: number } = $props();
  const app = useApp();

  let menuBtn = $state<HTMLButtonElement>()!;
  let menuOpen = $state(false);
  let editing = $state(false);
  let draft = $state("");
  let deleting = $state(false);
  let moveTo = $state("");

  const color = $derived(colorOr(column.color, CATEGORY_COLOR[column.category] ?? "#8e8e93"));
  const others = $derived(app.columns.filter((c) => c.id !== column.id));

  async function update(p: { title?: string; color?: string; category?: ColumnCategory; position?: number }) {
    try {
      const { column: c } = await app.call("column.update", { id: column.id, ...p });
      app.applyColumn(c);
    } catch {
      /* toast shown */
    }
  }

  function startRename() {
    menuOpen = false;
    draft = column.title;
    editing = true;
  }

  function commitRename() {
    const t = draft.trim();
    editing = false;
    if (t && t !== column.title) void update({ title: t });
  }

  function askDelete() {
    menuOpen = false;
    if (!others.length) {
      app.toast("A board needs at least one column.");
      return;
    }
    moveTo = others[Math.min(index, others.length - 1)]?.id ?? others[0].id;
    deleting = true;
  }

  async function doDelete() {
    deleting = false;
    try {
      await app.call("column.delete", { id: column.id, moveCardsTo: moveTo });
    } catch {
      /* toast shown */
    }
  }

  const CATS: ColumnCategory[] = ["unstarted", "started", "done", "closed"];
</script>

<header class="head" style="--c:{color}">
  <span class="dot"></span>
  {#if editing}
        <input
      class="field rename"
      bind:value={draft}
      use:focusOnMount
      onblur={commitRename}
      onkeydown={(e) => {
        if (e.key === "Enter") commitRename();
        if (e.key === "Escape") {
          e.stopPropagation();
          editing = false;
        }
      }}
    />
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <span class="title" ondblclick={startRename} title={CATEGORY_LABEL[column.category]}>{column.title}</span>
    <span class="count">{count}</span>
  {/if}
  <span class="spacer"></span>
  <button
    class="icon-btn"
    data-new-in={column.id}
    title="New session in {column.title}"
    onclick={(e) => (app.quick = { anchor: e.currentTarget as HTMLElement, columnId: column.id })}
  >
    <Icon name="plus" />
  </button>
  <button class="icon-btn" bind:this={menuBtn} title="Column options" onclick={() => (menuOpen = !menuOpen)}>
    <Icon name="more" />
  </button>
</header>

{#if menuOpen}
  <Popover anchor={menuBtn} onclose={() => (menuOpen = false)} width={210}>
    <button class="mi" onclick={startRename}><Icon name="pencil" size={14} />Rename</button>
    <div class="sep"></div>
    <div class="sub">Colour</div>
    <Swatches
      value={color}
      onpick={(c) => {
        menuOpen = false;
        void update({ color: c });
      }}
    />
    <div class="sep"></div>
    <div class="sub">Category</div>
    {#each CATS as cat (cat)}
      <button
        class="mi"
        onclick={() => {
          menuOpen = false;
          void update({ category: cat });
        }}
      >
        <span style="width:14px;display:inline-grid;place-items:center">
          {#if column.category === cat}<Icon name="check" size={13} />{/if}
        </span>
        {CATEGORY_LABEL[cat]}
      </button>
    {/each}
    <div class="sep"></div>
    <button
      class="mi"
      disabled={index === 0}
      onclick={() => {
        menuOpen = false;
        void update({ position: index - 1 });
      }}><Icon name="chevronLeft" size={14} />Move left</button
    >
    <button
      class="mi"
      disabled={index >= total - 1}
      onclick={() => {
        menuOpen = false;
        void update({ position: index + 1 });
      }}><Icon name="chevronRight" size={14} />Move right</button
    >
    <div class="sep"></div>
    <button class="mi danger" onclick={askDelete}><Icon name="trash" size={14} />Delete column…</button>
  </Popover>
{/if}

{#if deleting}
  <Dialog label="Delete column" onclose={() => (deleting = false)} width={380}>
    <div class="confirm">
      <h3>Delete “{column.title}”?</h3>
      {#if count > 0}
        <p>Its {count} card{count === 1 ? "" : "s"} will move to:</p>
        <Select
          label="Move cards to"
          value={moveTo}
          options={others.map((o) => ({ value: o.id, label: o.title, color: colorOr(o.color, CATEGORY_COLOR[o.category]) }))}
          onchange={(v) => (moveTo = v)}
        />
      {:else}
        <p>The column is empty.</p>
      {/if}
      <div class="actions">
        <button class="btn" onclick={() => (deleting = false)} data-autofocus>Cancel</button>
        <button class="btn primary" style="background:var(--danger)" onclick={doDelete}>Delete</button>
      </div>
    </div>
  </Dialog>
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 34px;
    padding: 0 6px 0 10px;
    flex: none;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--c);
    flex: none;
  }
  .title {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .count {
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .spacer {
    flex: 1;
  }
  .rename {
    height: 24px;
    padding: 2px 6px;
  }
  .confirm {
    padding: 18px 20px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
  }
  p {
    margin: 0;
    color: var(--text-2);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
</style>
