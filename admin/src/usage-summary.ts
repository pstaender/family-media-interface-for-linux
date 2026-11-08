// Groups usage statistics for Parental control → Usage: per day, per category, with the
// single entries for the detailed view. Pure (also used in the browser).

export const CATEGORY_ORDER = ["video", "disc", "music", "pictures", "website", "game", "program", "emulator"] as const;
export type UsageCategory = (typeof CATEGORY_ORDER)[number];

export interface UsageEntry {
  from: number;
  until: number;
  home_screen: string;
  category: UsageCategory;
  item: string;
  detail: string;
}

export interface CategorySummary {
  category: UsageCategory;
  secs: number;
  entries: UsageEntry[];
}

export interface DaySummary {
  /** `YYYY-MM-DD`, local time of the entries' start. */
  day: string;
  secs: number;
  categories: CategorySummary[];
}

function localDay(unix: number): string {
  const d = new Date(unix * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Newest day first; categories in a fixed order; entries oldest first within a category. */
export function summarize(entries: UsageEntry[], homeScreen?: string): DaySummary[] {
  const days = new Map<string, Map<UsageCategory, CategorySummary>>();
  for (const entry of entries) {
    if (homeScreen && entry.home_screen !== homeScreen) continue;
    const day = localDay(entry.from);
    let categories = days.get(day);
    if (!categories) days.set(day, (categories = new Map()));
    let summary = categories.get(entry.category);
    if (!summary) categories.set(entry.category, (summary = { category: entry.category, secs: 0, entries: [] }));
    summary.secs += entry.until - entry.from;
    summary.entries.push(entry);
  }
  return [...days.entries()]
    .sort(([a], [b]) => (a < b ? 1 : -1))
    .map(([day, categories]) => {
      const list = CATEGORY_ORDER.flatMap((c) => categories.get(c) ?? []);
      for (const c of list) c.entries.sort((a, b) => a.from - b.from);
      return { day, secs: list.reduce((sum, c) => sum + c.secs, 0), categories: list };
    });
}
