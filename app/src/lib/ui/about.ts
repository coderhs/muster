// Text for the About section in Settings.

/** A commit date (ISO 8601) as a short local date and time, or "" when unknown. */
export function commitWhen(iso: string | undefined | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

/**
 * Whether the app and the daemon were built from the same commit. `null` when one of
 * them is unknown (a browser client has no app build; an old daemon reports no commit).
 */
export function sameBuild(appCommit: string | undefined | null, daemonCommit: string | undefined | null): boolean | null {
  const known = (c: string | undefined | null) => !!c && c !== "unknown";
  if (!known(appCommit) || !known(daemonCommit)) return null;
  return appCommit === daemonCommit;
}
