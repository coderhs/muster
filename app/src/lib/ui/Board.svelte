<script lang="ts">
  import { focusOnMount } from "./actions";
  import { tick } from "svelte";
  import type { Card, ColumnCategory, Id } from "../protocol/types";
  import { neighboursAt } from "../board/board";
  import { useApp } from "../state/context";
  import CardFace from "./CardFace.svelte";
  import ColumnHeader from "./ColumnHeader.svelte";
  import Icon from "./Icon.svelte";

  const app = useApp();

  // ---------- drag and drop (pointer events: works with mouse, pen and touch) ----------
  interface Drag {
    card: Card;
    pointerId: number;
    startX: number;
    startY: number;
    dx: number;
    dy: number;
    x: number;
    y: number;
    w: number;
    h: number;
    active: boolean;
    overCol: Id | null;
    index: number;
    touchTimer: ReturnType<typeof setTimeout> | null;
  }
  let drag = $state<Drag | null>(null);
  let boardEl: HTMLDivElement;
  let scrollRaf = 0;

  function onCardPointerDown(e: PointerEvent, card: Card) {
    if (e.button !== 0 || drag) return;
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    drag = {
      card,
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      dx: e.clientX - r.left,
      dy: e.clientY - r.top,
      x: e.clientX,
      y: e.clientY,
      w: r.width,
      h: r.height,
      active: false,
      overCol: null,
      index: 0,
      touchTimer: null,
    };
    if (e.pointerType === "touch") {
      // On touch, a short hold starts the drag so the board can still scroll.
      drag.touchTimer = setTimeout(() => drag && activate(), 280);
    }
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", cancelDrag);
    window.addEventListener("keydown", onDragKey, true);
  }

  function activate() {
    if (!drag) return;
    drag.active = true;
    app.selectedCardId = drag.card.id;
    hitTest();
    autoScroll();
  }

  function onMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    drag.x = e.clientX;
    drag.y = e.clientY;
    if (!drag.active) {
      const moved = Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY);
      if (drag.touchTimer) {
        if (moved > 8) cancelDrag();
        return;
      }
      if (moved < 4) return;
      activate();
    }
    e.preventDefault();
    hitTest();
  }

  function hitTest() {
    if (!drag) return;
    const cols = boardEl.querySelectorAll<HTMLElement>("[data-col-id]");
    let over: HTMLElement | null = null;
    for (const c of cols) {
      const r = c.getBoundingClientRect();
      if (drag.x >= r.left && drag.x <= r.right) over = c;
    }
    if (!over) {
      drag.overCol = null;
      return;
    }
    const colId = over.dataset.colId!;
    const items = [...over.querySelectorAll<HTMLElement>("[data-card-id]")].filter(
      (x) => x.dataset.cardId !== drag!.card.id,
    );
    let index = items.length;
    for (let i = 0; i < items.length; i++) {
      const r = items[i].getBoundingClientRect();
      if (drag.y < r.top + r.height / 2) {
        index = i;
        break;
      }
    }
    drag.overCol = colId;
    drag.index = index;
  }

  function autoScroll() {
    cancelAnimationFrame(scrollRaf);
    const step = () => {
      if (!drag?.active) return;
      const edge = 48;
      const br = boardEl.getBoundingClientRect();
      if (drag.x < br.left + edge) boardEl.scrollLeft -= 12;
      else if (drag.x > br.right - edge) boardEl.scrollLeft += 12;
      const list = drag.overCol
        ? boardEl.querySelector<HTMLElement>(`[data-col-id="${CSS.escape(drag.overCol)}"] .cards`)
        : null;
      if (list) {
        const lr = list.getBoundingClientRect();
        if (drag.y < lr.top + edge) list.scrollTop -= 10;
        else if (drag.y > lr.bottom - edge) list.scrollTop += 10;
      }
      hitTest();
      scrollRaf = requestAnimationFrame(step);
    };
    scrollRaf = requestAnimationFrame(step);
  }

  function onUp(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    const d = drag;
    endDrag();
    if (!d.active || !d.overCol) return;
    const list = app.byColumn.get(d.overCol) ?? [];
    const current = list.findIndex((c) => c.id === d.card.id);
    const without = list.filter((c) => c.id !== d.card.id);
    if (d.overCol === d.card.columnId && current === d.index) return;
    const { before, after } = neighboursAt(without, d.index, d.card.id);
    void app.moveCard(d.card.id, d.overCol, before, after);
  }

  function onDragKey(e: KeyboardEvent) {
    if (e.key === "Escape" && drag) {
      e.preventDefault();
      e.stopPropagation();
      cancelDrag();
    }
  }

  function cancelDrag() {
    endDrag();
  }

  function endDrag() {
    if (drag?.touchTimer) clearTimeout(drag.touchTimer);
    cancelAnimationFrame(scrollRaf);
    drag = null;
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointercancel", cancelDrag);
    window.removeEventListener("keydown", onDragKey, true);
  }

  // Suppress the click that follows a drag.
  function onClickCapture(e: MouseEvent) {
    if (justDragged) {
      e.stopPropagation();
      e.preventDefault();
      justDragged = false;
    }
  }
  let justDragged = false;
  $effect(() => {
    if (drag?.active) justDragged = true;
    else if (!drag) setTimeout(() => (justDragged = false), 0);
  });

  // ---------- column reordering by header drag ----------
  interface ColDrag {
    id: Id;
    from: number;
    pointerId: number;
    startX: number;
    x: number;
    active: boolean;
    index: number;
  }
  let colDrag = $state<ColDrag | null>(null);

  function onColPointerDown(e: PointerEvent, id: Id, from: number) {
    if (e.button !== 0 || drag || colDrag) return;
    const t = e.target as HTMLElement;
    if (!t.closest(".head") || t.closest("button, input, select")) return;
    colDrag = { id, from, pointerId: e.pointerId, startX: e.clientX, x: e.clientX, active: false, index: from };
    window.addEventListener("pointermove", onColMove);
    window.addEventListener("pointerup", onColUp);
    window.addEventListener("pointercancel", endColDrag);
  }

  function onColMove(e: PointerEvent) {
    if (!colDrag || e.pointerId !== colDrag.pointerId) return;
    colDrag.x = e.clientX;
    if (!colDrag.active) {
      if (Math.abs(e.clientX - colDrag.startX) < 5) return;
      colDrag.active = true;
    }
    e.preventDefault();
    // Target index among the other columns, by their horizontal midpoints.
    const others = [...boardEl.querySelectorAll<HTMLElement>("[data-col-id]")].filter(
      (el) => el.dataset.colId !== colDrag!.id,
    );
    let index = others.length;
    for (let i = 0; i < others.length; i++) {
      const r = others[i].getBoundingClientRect();
      if (e.clientX < r.left + r.width / 2) {
        index = i;
        break;
      }
    }
    colDrag.index = index;
    const br = boardEl.getBoundingClientRect();
    if (e.clientX < br.left + 40) boardEl.scrollLeft -= 16;
    else if (e.clientX > br.right - 40) boardEl.scrollLeft += 16;
  }

  async function onColUp(e: PointerEvent) {
    if (!colDrag || e.pointerId !== colDrag.pointerId) return;
    const d = colDrag;
    endColDrag();
    if (!d.active) return;
    justDragged = true;
    setTimeout(() => (justDragged = false), 0);
    if (d.index === d.from) return;
    try {
      const { column } = await app.call("column.update", { id: d.id, position: d.index });
      app.applyColumn(column);
    } catch {
      /* toast shown */
    }
  }

  function endColDrag() {
    colDrag = null;
    window.removeEventListener("pointermove", onColMove);
    window.removeEventListener("pointerup", onColUp);
    window.removeEventListener("pointercancel", endColDrag);
  }

  /** Where the drop marker goes: before the column at this index of the full list. */
  const colMarker = $derived.by(() => {
    if (!colDrag?.active) return -1;
    const others = app.columns.filter((c) => c.id !== colDrag!.id);
    const target = others[colDrag.index];
    return target ? app.columns.findIndex((c) => c.id === target.id) : app.columns.length;
  });

  // ---------- keyboard ----------
  function locate(id: Id | null): { col: number; row: number } | null {
    if (!id) return null;
    for (let c = 0; c < app.columns.length; c++) {
      const row = (app.byColumn.get(app.columns[c].id) ?? []).findIndex((x) => x.id === id);
      if (row >= 0) return { col: c, row };
    }
    return null;
  }

  function cardAt(col: number, row: number): Card | null {
    const list = app.byColumn.get(app.columns[col]?.id) ?? [];
    if (!list.length) return null;
    return list[Math.max(0, Math.min(row, list.length - 1))];
  }

  async function select(id: Id | null) {
    app.selectedCardId = id;
    await tick();
    if (!id) return;
    const el = boardEl?.querySelector<HTMLElement>(`[data-card-id="${CSS.escape(id)}"]`);
    el?.scrollIntoView({ block: "nearest", inline: "nearest" });
    el?.focus({ preventScroll: true });
  }

  function firstCard(): Card | null {
    for (const c of app.columns) {
      const l = app.byColumn.get(c.id);
      if (l?.length) return l[0];
    }
    return null;
  }

  function moveSelection(dc: number, dr: number) {
    const at = locate(app.selectedCardId);
    if (!at) return void select(firstCard()?.id ?? null);
    if (dr) {
      const next = cardAt(at.col, at.row + dr);
      if (next) void select(next.id);
      return;
    }
    // Skip empty columns when moving sideways.
    for (let c = at.col + dc; c >= 0 && c < app.columns.length; c += dc) {
      const next = cardAt(c, at.row);
      if (next) return void select(next.id);
    }
  }

  function moveSelectedCard(dc: number, dr: number) {
    const at = locate(app.selectedCardId);
    if (!at) return;
    const card = cardAt(at.col, at.row)!;
    if (dr) {
      const list = app.byColumn.get(app.columns[at.col].id) ?? [];
      const target = at.row + dr;
      if (target < 0 || target >= list.length) return;
      const without = list.filter((c) => c.id !== card.id);
      const { before, after } = neighboursAt(without, target, card.id);
      void app.moveCard(card.id, card.columnId, before, after).then(() => select(card.id));
    } else {
      const col = app.columns[at.col + dc];
      if (!col) return;
      const list = (app.byColumn.get(col.id) ?? []).filter((c) => c.id !== card.id);
      const { before, after } = neighboursAt(list, Math.min(at.row, list.length), card.id);
      void app.moveCard(card.id, col.id, before, after).then(() => select(card.id));
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.defaultPrevented || app.openCardId || app.newCard || app.quick || app.projectSettings || app.shortcutsOpen || app.settingsOpen || drag) return;
    const t = e.target as HTMLElement;
    if (t.closest("input, textarea, select, [contenteditable], [role=dialog], [role=menu]")) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    const shift = e.shiftKey;
    const map: Record<string, [number, number]> = {
      ArrowLeft: [-1, 0],
      ArrowRight: [1, 0],
      ArrowUp: [0, -1],
      ArrowDown: [0, 1],
      h: [-1, 0],
      l: [1, 0],
      k: [0, -1],
      j: [0, 1],
      H: [-1, 0],
      L: [1, 0],
      K: [0, -1],
      J: [0, 1],
    };
    if (map[k]) {
      e.preventDefault();
      const [dc, dr] = map[k];
      if (shift) moveSelectedCard(dc, dr);
      else moveSelection(dc, dr);
    } else if (k === "Enter" && app.selectedCardId) {
      e.preventDefault();
      app.openCardId = app.selectedCardId;
    } else if (k === "n" || k === "c" || k === "N" || k === "C") {
      e.preventDefault();
      const at = locate(app.selectedCardId);
      const columnId = app.defaultColumn(at ? app.columns[at.col].id : null);
      const anchor = boardEl.querySelector<HTMLElement>(`[data-new-in="${CSS.escape(columnId ?? "")}"]`);
      // N: pick an agent and go; Shift+N: the full dialog.
      if (shift || !anchor) app.newCard = { columnId };
      else app.quick = { anchor, columnId };
    } else if (k === "s" && app.selectedCardId) {
      const c = app.card(app.selectedCardId);
      if (c?.status === "draft") {
        e.preventDefault();
        void app.startCard(c.id).catch(() => {});
      }
    } else if (k === "Escape" && app.selectedCardId) {
      app.selectedCardId = null;
    }
  }

  // ---------- add column ----------
  let adding = $state(false);
  let newTitle = $state("");
  async function addColumn() {
    const title = newTitle.trim();
    adding = false;
    newTitle = "";
    if (!title || !app.currentProjectId) return;
    try {
      const { column } = await app.call("column.create", {
        projectId: app.currentProjectId,
        title,
        category: "started" as ColumnCategory,
      });
      app.applyColumn(column);
    } catch {
      /* toast shown */
    }
  }

  function open(card: Card) {
    app.selectedCardId = card.id;
    app.openCardId = card.id;
  }
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="board" bind:this={boardEl} class:dragging={drag?.active || colDrag?.active} onclickcapture={onClickCapture}>
  {#each app.columns as column, ci (column.id)}
    {@const cards = app.byColumn.get(column.id) ?? []}
    {@const visible = drag?.active ? cards.filter((c) => c.id !== drag?.card.id) : cards}
    {@const dropHere = !!drag?.active && drag.overCol === column.id}
    {#if colMarker === ci}<div class="col-marker"></div>{/if}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <section
      class="column"
      data-col-id={column.id}
      class:over={drag?.active && drag.overCol === column.id}
      class:col-dragging={colDrag?.active && colDrag.id === column.id}
      onpointerdown={(e) => onColPointerDown(e, column.id, ci)}
    >
      <ColumnHeader {column} count={cards.length} index={ci} total={app.columns.length} />
      <div class="cards" role="listbox" aria-label={column.title}>
        {#each visible as card, i (card.id)}
          {#if dropHere && drag?.index === i}
            <div class="placeholder" style="height:{drag.h}px"></div>
          {/if}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="slot" onpointerdown={(e) => onCardPointerDown(e, card)}>
            <CardFace
              {card}
              agent={app.agent(card.agentId)}
              selected={app.selectedCardId === card.id}
              onselect={() => void select(card.id)}
              onopen={() => open(card)}
              onstart={() => void app.startCard(card.id).catch(() => {})}
            />
          </div>
        {/each}
        {#if dropHere && drag && drag.index >= visible.length}
          <div class="placeholder" style="height:{drag.h}px"></div>
        {/if}
        {#if !cards.length && !(drag?.active && drag.overCol === column.id)}
          <button class="empty" onclick={(e) => (app.quick = { anchor: e.currentTarget as HTMLElement, columnId: column.id })}>
            <Icon name="plus" size={14} /> New session
          </button>
        {/if}
      </div>
    </section>
  {/each}

  {#if colMarker === app.columns.length}<div class="col-marker"></div>{/if}
  <div class="add-col">
    {#if adding}
            <input
        class="field"
        placeholder="Column title"
        bind:value={newTitle}
        use:focusOnMount
        onblur={addColumn}
        onkeydown={(e) => {
          if (e.key === "Enter") addColumn();
          if (e.key === "Escape") {
            e.stopPropagation();
            adding = false;
            newTitle = "";
          }
        }}
      />
    {:else}
      <button class="add-btn" onclick={() => (adding = true)}><Icon name="plus" size={14} />Add column</button>
    {/if}
  </div>
</div>

{#if drag?.active}
  <div class="ghost" style="width:{drag.w}px; transform: translate({drag.x - drag.dx}px, {drag.y - drag.dy}px) rotate(1.5deg)">
    <CardFace card={drag.card} agent={app.agent(drag.card.agentId)} onopen={() => {}} onstart={() => {}} onselect={() => {}} />
  </div>
{/if}

<style>
  .board {
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: stretch;
    gap: 10px;
    padding: 4px 14px 14px;
    overflow-x: auto;
    overflow-y: hidden;
  }
  .board.dragging {
    cursor: grabbing;
  }
  .board.dragging :global(*) {
    cursor: grabbing !important;
  }
  .column {
    flex: 0 0 280px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-column);
    border-radius: 10px;
    border: 1px solid var(--border);
    transition: background 0.1s;
  }
  .column :global(.head) {
    cursor: grab;
  }
  .column.col-dragging {
    opacity: 0.4;
  }
  .col-marker {
    flex: 0 0 3px;
    margin: 0 -6px;
    border-radius: 2px;
    background: var(--accent);
  }
  .column.over {
    background: color-mix(in srgb, var(--accent) 7%, var(--bg-column));
  }
  .cards {
    flex: 1;
    min-height: 40px;
    overflow-y: auto;
    padding: 2px 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .placeholder {
    flex: none;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border: 1.5px dashed color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .empty {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 36px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-3);
  }
  .empty:hover {
    color: var(--text-2);
    background: var(--bg-hover);
  }
  .add-col {
    flex: 0 0 240px;
    padding-top: 2px;
  }
  .add-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 34px;
    padding: 0 10px;
    border-radius: 10px;
    border: 1px dashed var(--border-strong);
    background: transparent;
    color: var(--text-2);
  }
  .add-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .ghost {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 200;
    pointer-events: none;
    filter: drop-shadow(0 12px 24px rgba(0, 0, 0, 0.35));
    opacity: 0.95;
  }
</style>
