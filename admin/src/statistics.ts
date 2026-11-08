// Usage statistics, written by the media center (src/stats.rs): one JSON line per
// activity in $XDG_DATA_HOME/curator/statistics/<YYYY-MM-DD>.jsonl. The admin panel only
// reads them (Parental control → Usage) and can delete them all; only files named like
// ours are touched.

import { readFile, readdir, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

export function defaultStatisticsDir(): string {
  const data = process.env.XDG_DATA_HOME || join(homedir(), ".local", "share");
  return join(data, "curator", "statistics");
}

export const CATEGORIES = ["disc", "video", "music", "pictures", "website", "game"] as const;
export type Category = (typeof CATEGORIES)[number];

export interface StatisticsEntry {
  /** Unix times. */
  from: number;
  until: number;
  home_screen: string;
  category: Category;
  item: string;
  detail: string;
}

const DAY_FILE = /^(\d{4}-\d{2}-\d{2})\.jsonl$/;
const MAX_FILE_BYTES = 8 * 1024 * 1024;

function text(value: unknown, max: number): string | null {
  return typeof value === "string" && value.length <= max ? value : null;
}

/** One line, checked (the file is ours, but read it as untrusted data anyway). */
export function parseEntry(line: string): StatisticsEntry | null {
  let raw: unknown;
  try {
    raw = JSON.parse(line);
  } catch {
    return null;
  }
  if (typeof raw !== "object" || raw === null) return null;
  const e = raw as Record<string, unknown>;
  const from = e.from;
  const until = e.until;
  if (!Number.isSafeInteger(from) || !Number.isSafeInteger(until) || (until as number) < (from as number)) return null;
  if (!(CATEGORIES as readonly unknown[]).includes(e.category)) return null;
  const screen = text(e.home_screen, 40);
  const item = text(e.item, 500);
  const detail = e.detail === undefined ? "" : text(e.detail, 2000);
  if (screen === null || item === null || detail === null) return null;
  return { from: from as number, until: until as number, home_screen: screen, category: e.category as Category, item, detail };
}

/** `YYYY-MM-DD` of a date, local time. */
function day(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** The entries of the last `days` days (today included), oldest first. */
export async function readStatistics(dir: string, days: number, now = new Date()): Promise<StatisticsEntry[]> {
  const first = new Date(now);
  first.setDate(first.getDate() - (days - 1));
  const from = day(first);
  const names = (await readdir(dir).catch(() => [] as string[]))
    .filter((n) => DAY_FILE.exec(n) && n.slice(0, 10) >= from)
    .sort();
  const entries: StatisticsEntry[] = [];
  for (const name of names) {
    const content = await readFile(join(dir, name), "utf8").catch(() => "");
    if (content.length > MAX_FILE_BYTES) continue;
    for (const line of content.split("\n")) {
      const entry = line.trim() ? parseEntry(line) : null;
      if (entry) entries.push(entry);
    }
  }
  return entries.sort((a, b) => a.from - b.from);
}

/** Delete all statistics files (only `YYYY-MM-DD.jsonl`). Returns how many. */
export async function deleteStatistics(dir: string): Promise<number> {
  let deleted = 0;
  for (const name of await readdir(dir).catch(() => [] as string[])) {
    if (!DAY_FILE.test(name)) continue;
    await rm(join(dir, name), { force: true });
    deleted++;
  }
  return deleted;
}
