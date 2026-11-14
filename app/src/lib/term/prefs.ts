// Terminal appearance, kept per browser profile (localStorage) and applied live.

export interface TermPrefs {
  fontSize: number;
  fontFamily: string;
  lineHeight: number;
  bold: boolean;
}

export const FONT_CHOICES: Array<{ value: string; label: string }> = [
  { value: '"Hack Nerd Font Mono", "SF Mono", ui-monospace, Menlo, monospace', label: "Hack Nerd Font Mono (if installed)" },
  { value: '"SF Mono", ui-monospace, Menlo, monospace', label: "SF Mono" },
  { value: "Menlo, monospace", label: "Menlo" },
  { value: "Monaco, monospace", label: "Monaco" },
  { value: '"JetBrains Mono", "SF Mono", Menlo, monospace', label: "JetBrains Mono (if installed)" },
  { value: '"Fira Code", "SF Mono", Menlo, monospace', label: "Fira Code (if installed)" },
];

const KEY = "muster.term";
const DEFAULTS: TermPrefs = { fontSize: 14, fontFamily: FONT_CHOICES[0].value, lineHeight: 1.15, bold: true };
const listeners = new Set<(p: TermPrefs) => void>();

export function getTermPrefs(): TermPrefs {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return {
      fontSize: clamp(Number(v.fontSize) || DEFAULTS.fontSize, 9, 24),
      fontFamily: typeof v.fontFamily === "string" && v.fontFamily ? v.fontFamily : DEFAULTS.fontFamily,
      lineHeight: clamp(Number(v.lineHeight) || DEFAULTS.lineHeight, 1, 1.6),
      bold: typeof v.bold === "boolean" ? v.bold : DEFAULTS.bold,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

export function setTermPrefs(p: Partial<TermPrefs>) {
  const next = { ...getTermPrefs(), ...p };
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* storage disabled: apply for this session only */
  }
  for (const f of listeners) f(next);
}

export function onTermPrefs(f: (p: TermPrefs) => void): () => void {
  listeners.add(f);
  return () => listeners.delete(f);
}

const clamp = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, n));
