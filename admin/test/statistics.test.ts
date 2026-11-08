import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseEntry, readStatistics } from "../src/statistics";
import { summarize, type UsageEntry } from "../src/usage-summary";

const good = { from: 10, until: 20, home_screen: "kids", category: "game", item: "Ping Pong" };

test("entries are checked", () => {
  expect(parseEntry(JSON.stringify(good))).toEqual({ ...good, detail: "" } as never);
  for (const bad of [
    "nonsense",
    "null",
    JSON.stringify({ ...good, category: "shell" }),
    JSON.stringify({ ...good, until: 5 }),
    JSON.stringify({ ...good, from: "x" }),
    JSON.stringify({ ...good, item: 7 }),
    JSON.stringify({ ...good, home_screen: "x".repeat(41) }),
  ]) {
    expect(parseEntry(bad)).toBeNull();
  }
});

test("only the asked days are read, oldest first", async () => {
  const dir = mkdtempSync("/tmp/curator-stats-");
  const line = (from: number) => `${JSON.stringify({ ...good, from, until: from + 1 })}\n`;
  writeFileSync(join(dir, "2026-11-01.jsonl"), line(1));
  writeFileSync(join(dir, "2026-11-07.jsonl"), line(3));
  writeFileSync(join(dir, "2026-11-08.jsonl"), line(2));
  writeFileSync(join(dir, "other.jsonl"), line(9));
  const now = new Date(2026, 10, 8, 12);
  expect((await readStatistics(dir, 2, now)).map((e) => e.from)).toEqual([2, 3]);
  expect((await readStatistics(dir, 1, now)).map((e) => e.from)).toEqual([2]);
  expect((await readStatistics(dir, 8, now)).length).toBe(3);
  expect(await readStatistics("/nonexistent/dir", 7, now)).toEqual([]);
});

test("usage is grouped by day and category", () => {
  const at = (day: number, hour: number) => new Date(2026, 10, day, hour).getTime() / 1000;
  const entry = (from: number, secs: number, category: UsageEntry["category"], home_screen = "kids"): UsageEntry => ({
    from,
    until: from + secs,
    home_screen,
    category,
    item: "x",
    detail: "",
  });
  const days = summarize([entry(at(7, 10), 60, "game"), entry(at(8, 9), 30, "music"), entry(at(8, 8), 10, "music"), entry(at(8, 7), 5, "video", "family")]);
  expect(days.map((d) => [d.day, d.secs])).toEqual([["2026-11-08", 45], ["2026-11-07", 60]]);
  expect(days[0]!.categories.map((c) => [c.category, c.secs])).toEqual([["video", 5], ["music", 40]]);
  expect(days[0]!.categories[1]!.entries.map((e) => e.until - e.from)).toEqual([10, 30]);
  expect(summarize([entry(at(8, 7), 5, "video", "family")], "kids")).toEqual([]);
});
