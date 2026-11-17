// Unsent review comments, per card. Kept in memory and in this browser's storage, so a
// review survives closing the card or reloading the window until it is sent.

import { refKey, type LineRef, type ReviewComment } from "./patch";

interface Draft {
  comments: ReviewComment[];
  note: string;
}

const KEY = (cardId: string) => `muster.review.${cardId}`;
const drafts = $state<Record<string, Draft>>({});

function load(cardId: string): Draft {
  if (!drafts[cardId]) {
    let d: Draft = { comments: [], note: "" };
    try {
      const raw = localStorage.getItem(KEY(cardId));
      if (raw) {
        const v = JSON.parse(raw);
        if (Array.isArray(v?.comments)) d = { comments: v.comments, note: typeof v.note === "string" ? v.note : "" };
      }
    } catch {
      /* storage unavailable or bad JSON: start empty */
    }
    drafts[cardId] = d;
  }
  return drafts[cardId];
}

function save(cardId: string) {
  const d = drafts[cardId];
  try {
    if (!d || (!d.comments.length && !d.note.trim())) localStorage.removeItem(KEY(cardId));
    else localStorage.setItem(KEY(cardId), JSON.stringify(d));
  } catch {
    /* ignore */
  }
}

export function review(cardId: string) {
  const d = load(cardId);
  return {
    get comments() {
      return d.comments;
    },
    get note() {
      return d.note;
    },
    set note(v: string) {
      d.note = v;
      save(cardId);
    },
    find(ref: LineRef) {
      const k = refKey(ref);
      return d.comments.find((c) => refKey(c) === k);
    },
    /** Adds, replaces, or with empty text removes the comment on a line. */
    put(c: ReviewComment) {
      const k = refKey(c);
      const i = d.comments.findIndex((x) => refKey(x) === k);
      if (!c.text.trim()) {
        if (i >= 0) d.comments.splice(i, 1);
      } else if (i >= 0) d.comments[i] = c;
      else d.comments.push(c);
      save(cardId);
    },
    clear() {
      d.comments = [];
      d.note = "";
      save(cardId);
    },
  };
}
