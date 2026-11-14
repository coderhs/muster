// Short labels for the collapsed projects rail.

const chars = (name: string) => [...name.trim().replace(/[^\p{L}\p{N}]/gu, "")];

/**
 * One label per name: its first letter, or its first two when another name starts
 * with the same letter. Letter case is ignored when comparing.
 */
export function railLabels(names: string[]): string[] {
  const firsts = names.map((n) => (chars(n)[0] ?? "").toLowerCase());
  return names.map((n, i) => {
    const cs = chars(n);
    if (!cs.length) return "?";
    const shared = firsts.some((f, j) => j !== i && f === firsts[i]);
    const first = cs[0].toUpperCase();
    return shared && cs[1] ? first + cs[1].toLowerCase() : first;
  });
}

/** Black or white, whichever reads better on `hex` (#rgb or #rrggbb). */
export function inkOn(hex: string): string {
  let h = hex.replace("#", "");
  if (h.length === 3 || h.length === 4) h = [...h].map((c) => c + c).join("");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
  const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return lum > 0.6 ? "#1d1d1f" : "#ffffff";
}
