/** Focus (and select) an element once it is in the document. */
export function focusOnMount(node: HTMLInputElement | HTMLTextAreaElement) {
  requestAnimationFrame(() => {
    node.focus();
    node.select();
  });
}

/**
 * Move an element to the end of <body>. Popups use it so `position: fixed` is relative
 * to the viewport even when an ancestor (an animated dialog) has a transform, filter or
 * containment, which would otherwise become the containing block.
 */
export function portal(node: HTMLElement) {
  document.body.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}

export interface Placement {
  x: number;
  y: number;
  maxHeight: number;
  up: boolean;
}

/** Place a popup of size w×h under (or, without room, over) an anchor rect, kept on screen. */
export function placeBelow(anchor: DOMRect, w: number, h: number, gap = 4, margin = 8): Placement {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  let x = anchor.left;
  if (x + w > vw - margin) x = Math.max(margin, Math.min(anchor.right - w, vw - margin - w));
  x = Math.max(margin, x);
  const below = vh - anchor.bottom - gap - margin;
  const above = anchor.top - gap - margin;
  const up = h > below && above > below;
  const maxHeight = Math.max(80, up ? above : below);
  const y = up ? Math.max(margin, anchor.top - gap - Math.min(h, maxHeight)) : anchor.bottom + gap;
  return { x, y, maxHeight, up };
}
