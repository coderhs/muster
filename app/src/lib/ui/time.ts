/** "12s", "3m", "2h 5m", "4d". */
export function shortDuration(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h}h ${m % 60}m` : `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

/** Due-date chip text relative to `now`: "today", "tomorrow", "in 3d", "2d ago", or a date. */
export function dueLabel(dueAt: number, now: number): string {
  const day = 86_400_000;
  const startOf = (t: number) => {
    const d = new Date(t);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  };
  const diff = Math.round((startOf(dueAt) - startOf(now)) / day);
  if (diff === 0) return "today";
  if (diff === 1) return "tomorrow";
  if (diff === -1) return "yesterday";
  if (diff > 1 && diff < 7) return `in ${diff}d`;
  if (diff < -1 && diff > -7) return `${-diff}d ago`;
  return new Date(dueAt).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
