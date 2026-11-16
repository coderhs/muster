import { describe, expect, it } from "vitest";
import type { Card, Project } from "../protocol/types";
import { searchCards } from "./search";

const card = (id: string, extra: Partial<Card> = {}): Card => ({
  id,
  projectId: "a",
  columnId: "c",
  rank: "m",
  title: id,
  agentId: "claude",
  cwd: "/",
  status: "idle",
  statusSince: 0,
  unread: false,
  hasSession: true,
  createdAt: 0,
  updatedAt: 0,
  ...extra,
});

const project = (id: string, position: number): Project => ({ id, name: id, path: `/${id}`, color: "#fff", position });
const projects = [project("a", 0), project("b", 1), project("c", 2)];

const ids = (groups: ReturnType<typeof searchCards>) => groups.map((g) => [g.project.id, g.hits.map((h) => h.card.id)]);

describe("card search", () => {
  it("puts the current project first, then the others in sidebar order", () => {
    const cards = [
      card("login a", { projectId: "a" }),
      card("login b", { projectId: "b" }),
      card("login c", { projectId: "c" }),
    ];
    expect(ids(searchCards(cards, projects, "login", "b"))).toEqual([
      ["b", ["login b"]],
      ["a", ["login a"]],
      ["c", ["login c"]],
    ]);
  });

  it("needs every word, in any field and any order, ignoring case", () => {
    const cards = [
      card("Fix login", { notes: "Safari only" }),
      card("Fix logout"),
      card("Docs", { branch: "feat/login-safari" }),
    ];
    expect(ids(searchCards(cards, projects, "safari LOGIN", null))).toEqual([["a", ["Fix login", "Docs"]]]);
  });

  it("ranks title starts, then word starts, then inside words, then other fields", () => {
    const cards = [
      card("notes hit", { notes: "deploy" }),
      card("redeploy"),
      card("run deploy"),
      card("deploy script"),
    ];
    expect(ids(searchCards(cards, projects, "deploy", null))).toEqual([
      ["a", ["deploy script", "run deploy", "redeploy", "notes hit"]],
    ]);
  });

  it("breaks ties by the most recent activity", () => {
    const cards = [card("old", { title: "api", updatedAt: 1 }), card("new", { title: "api", lastActivityAt: 9 })];
    expect(ids(searchCards(cards, projects, "api", null))).toEqual([["a", ["new", "old"]]]);
  });

  it("shows an excerpt only when the title does not hold the match", () => {
    const cards = [card("Fix login"), card("Other", { prompt: "Make the login page load faster on slow networks" })];
    const [g] = searchCards(cards, projects, "login", null);
    expect(g.hits.map((h) => h.excerpt)).toEqual([null, "Make the login page load faster on slow networks"]);
  });

  it("lists recent cards per project for an empty query", () => {
    const cards = Array.from({ length: 8 }, (_, i) => card(`c${i}`, { updatedAt: i }));
    const [g] = searchCards(cards, projects, "  ", "a");
    expect(g.hits.map((h) => h.card.id)).toEqual(["c7", "c6", "c5", "c4", "c3"]);
  });

  it("leaves out projects with no match", () => {
    expect(searchCards([card("x", {})], projects, "nothing", "a")).toEqual([]);
  });
});
