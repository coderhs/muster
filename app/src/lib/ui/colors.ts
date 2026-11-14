import type { ColumnCategory } from "../protocol/types";

/** macOS system palette, used for projects and columns. */
export const PALETTE = [
  "#8e8e93",
  "#0a84ff",
  "#64d2ff",
  "#30d158",
  "#ffd60a",
  "#ff9f0a",
  "#ff453a",
  "#ff375f",
  "#bf5af2",
  "#5e5ce6",
  "#ac8e68",
];

export const CATEGORY_COLOR: Record<ColumnCategory, string> = {
  unstarted: "#8e8e93",
  started: "#0a84ff",
  done: "#30d158",
  closed: "#5e5ce6",
};

export const CATEGORY_LABEL: Record<ColumnCategory, string> = {
  unstarted: "Not started",
  started: "In progress",
  done: "Done",
  closed: "Closed",
};

export const colorOr = (c: string | null | undefined, fallback: string) =>
  c && /^#[0-9a-f]{3,8}$/i.test(c) ? c : fallback;
