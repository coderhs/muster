// Changed images in the Changes tab: which files are shown as pictures, and their sizes.

/** Extensions the daemon's `card.image` accepts (crates/muster-core/src/changes.rs). */
const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico", "svg"]);

export function isImage(path: string): boolean {
  const dot = path.lastIndexOf(".");
  return dot >= 0 && IMAGE_EXT.has(path.slice(dot + 1).toLowerCase());
}

/** "812 B", "14.2 KB", "3.1 MB". */
export function bytesLabel(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** "+12%", "−40%", "×2.5" or "×23" past doubling, or "" when the size did not change
 *  (or one side is missing). */
export function sizeChange(before?: number, after?: number): string {
  if (!before || after == null || before === after) return "";
  const ratio = after / before;
  if (ratio >= 2) return `×${ratio >= 10 ? Math.round(ratio) : ratio.toFixed(1)}`;
  const pct = Math.round((ratio - 1) * 100);
  if (pct === 0) return "";
  return pct > 0 ? `+${pct}%` : `−${-pct}%`;
}
