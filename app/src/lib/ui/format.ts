import type { ProcStats } from "../protocol/types";

export function bytes(n: number): string {
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(n >= 10 * 1024 ** 3 ? 0 : 1)} GB`;
  if (n >= 1024 ** 2) return `${Math.round(n / 1024 ** 2)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
}

export function cpu(p: number): string {
  return p >= 10 ? `${Math.round(p)}%` : `${p.toFixed(1).replace(/\.0$/, "")}%`;
}

export function statsLine(s: ProcStats): string {
  return `${bytes(s.memBytes)} · ${cpu(s.cpuPercent)} CPU`;
}
