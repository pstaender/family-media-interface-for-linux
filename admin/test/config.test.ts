import { expect, test } from "bun:test";
import example from "../../docs/config.example.json";
import { addHomeScreen, cleanScreenName, defaultConfig, defaultScreen, makeDefaultScreen, removeHomeScreen, isNormalizedAbsolutePath, validateConfig, websiteDomain } from "../src/config";

test("the example config is valid", () => {
  const v = validateConfig(example);
  expect(v.ok).toBe(true);
  if (v.ok) expect(v.config.home_screens[0]!.discs.audio_languages).toEqual(["deu", "eng"]);
});

test("the example folders are valid", () => {
  const v = validateConfig(example);
  expect(v.ok && v.config.home_screens[0]!.folders.map((f) => f.id)).toEqual(["movies", "photos", "stories"]);
});

test("the screensaver settings", () => {
  const v = validateConfig(example);
  expect(v.ok && v.config.screensaver).toEqual({
    mode: "aerial",
    idle_minutes: 10,
    aerial_quality: "1080p_hevc",
    aerial_videos: ["B876B645-3955-420E-99DF-60139E451CF3", "6C3D54AE-0871-498A-81D0-56ED24E5FE9F"],
  });
  const d = validateConfig({ version: 1 });
  expect(d.ok && d.config.screensaver).toEqual({ mode: "title", idle_minutes: 10, aerial_quality: "1080p_h264", aerial_videos: [] });
});

test("the games settings, also from their old place", () => {
  const v = validateConfig(example);
  expect(v.ok && v.config.home_screens[0]!.games).toEqual({ pong: { enabled: true, computer_level: "easy", points_to_win: 5 }, bricks: { enabled: true, lives: 3, sound: true }, blocks: { enabled: true, start_level: 2, sound: false } });
  const d = validateConfig({ version: 1 });
  expect(d.ok && d.config.home_screens[0]!.games).toEqual({ pong: { enabled: false, computer_level: "normal", points_to_win: 7 }, bricks: { enabled: false, lives: 5, sound: false }, blocks: { enabled: false, start_level: 0, sound: false } });
  const old = validateConfig({ version: 1, programs: { pong: { enabled: true, points_to_win: 9 } } });
  expect(old.ok && old.config.home_screens[0]!.games.pong).toEqual({ enabled: true, computer_level: "normal", points_to_win: 9 });
  expect(old.ok && "programs" in old.config).toBe(false);
  const both = validateConfig({ version: 1, programs: { pong: { enabled: true } }, games: { pong: { enabled: false } } });
  expect(both.ok && both.config.home_screens[0]!.games.pong.enabled).toBe(false);
});

test("home screens", () => {
  const v = validateConfig(example);
  if (!v.ok) throw new Error(v.error);
  expect(v.config.home_screens.map((s) => [s.id, s.default])).toEqual([["family", true], ["kids", false]]);
  expect(defaultScreen(v.config).id).toBe("family");
  expect(v.config.home_screens[1]!.limits.length).toBe(1);
  expect(v.config.statistics).toEqual({ enabled: true, keep_days: 90 });
  // Written form is accepted again unchanged.
  const again = validateConfig(JSON.parse(JSON.stringify(v.config)));
  expect(again.ok && again.config).toEqual(v.config);
});

test("adding, starring and removing home screens", () => {
  const v = validateConfig(example);
  if (!v.ok) throw new Error(v.error);
  const added = addHomeScreen(v.config, "Kids");
  expect(added.id).toBe("kids-2"); // "kids" is taken
  const copy = added.config.home_screens.at(-1)!;
  const family = v.config.home_screens[0]!;
  expect(copy.default).toBe(false);
  expect(copy.name).toBe("Kids");
  expect(copy.folders.map((f) => f.path)).toEqual(family.folders.map((f) => f.path));
  expect(copy.limits).toEqual(family.limits);
  // The copy is valid: its folder and website ids are new.
  expect(validateConfig(added.config).ok).toBe(true);
  copy.limits.length = 0;
  expect(family.limits.length).toBe(4); // a real copy

  const starred = makeDefaultScreen(added.config, added.id);
  expect(starred.home_screens.filter((s) => s.default).map((s) => s.id)).toEqual([added.id]);
  expect(validateConfig(starred).ok).toBe(true);

  expect(removeHomeScreen(added.config, added.id).home_screens.length).toBe(2);
  expect(removeHomeScreen(added.config, "family")).toBe(added.config); // not the default one

  expect(cleanScreenName("  Kids\u0007 ")).toBe("Kids");
  expect(cleanScreenName("   ")).toBeNull();
  expect(cleanScreenName("x".repeat(31))).toBeNull();
});

test("version 1 becomes one home screen", () => {
  const v = validateConfig({
    version: 1,
    home: { title: "Kino" },
    folders: [{ id: "m", name: "M", path: "/m", video: true }],
    parental: { pin_backoff_doubling: false, limits: [{ type: "budget", minutes: 60, scope: { kind: "every_day" } }] },
    programs: { pong: { enabled: true } },
  });
  if (!v.ok) throw new Error(v.error);
  expect(v.config.version).toBe(2);
  const s = v.config.home_screens[0]!;
  expect([s.id, s.name, s.default]).toEqual(["home", "Home", true]);
  expect(s.home.title).toBe("Kino");
  expect(s.limits.length).toBe(1);
  expect(s.games.pong.enabled).toBe(true);
  expect(v.config.parental).toEqual({ pin_backoff_doubling: false });
});

test("the removed extensions option is ignored", () => {
  const v = validateConfig({ version: 1, folders: [{ id: "a", name: "A", path: "/x", video: true, extensions: ["pdf"] }] });
  expect(v.ok && v.config.home_screens[0]!.folders[0]).toEqual({ id: "a", name: "A", path: "/x", video: true, audio: false, images: false });
});

test("the example websites are valid", () => {
  const v = validateConfig(example);
  expect(v.ok && v.config.home_screens[0]!.websites.map((w) => [w.id, w.allow_subdomains])).toEqual([["kika", true], ["wiki", false]]);
  const w = validateConfig({ version: 1, websites: [{ id: "k", name: "K", url: "https://kika.de/" }] });
  expect(w.ok && w.config.home_screens[0]!.websites[0]!.allow_subdomains).toBe(true);
  expect(w.ok && w.config.home_screens[0]!.websites[0]!.show_logo).toBe(true);
  expect(v.ok && v.config.home_screens[0]!.websites[1]!.show_logo).toBe(false);
});

test("website domains like src/sites.rs", () => {
  expect(websiteDomain("https://www.kika.de/start", true)).toEqual({ host: "kika.de", subdomains: true });
  expect(websiteDomain("https://www.kika.de/", false)).toEqual({ host: "www.kika.de", subdomains: false });
  expect(websiteDomain("https://www.Müller.de/", true)).toEqual({ host: "xn--mller-kva.de", subdomains: true });
  for (const bad of ["http://127.1/", "https://[::1]/", "https://intranet/", "https://app.localhost/", "ftp://kika.de/"]) {
    expect(typeof websiteDomain(bad, true)).toBe("string");
  }
});

test("parental settings", () => {
  const v = validateConfig(example);
  expect(v.ok && v.config.home_screens[0]!.limits.length).toBe(4);
  expect(v.ok && v.config.home_screens[0]!.limits[1]).toEqual({
    type: "window",
    from: "07:00",
    until: "19:30",
    scope: { kind: "weekdays", days: ["mon", "tue", "wed", "thu", "fri"] },
  });
  const week = validateConfig({
    version: 1,
    parental: { limits: [{ type: "budget", minutes: 2000, scope: { kind: "this_week", week_of: "2026-11-02" } }] },
  });
  expect(week.ok).toBe(true);
  const def = validateConfig({ version: 1 });
  expect(def.ok && def.config.parental).toEqual({ pin_backoff_doubling: true });
});

test("home title", () => {
  const v = validateConfig(example);
  expect(v.ok && v.config.home_screens[0]!.home).toEqual({ title: "Family Cinema", show_title: true, language: "de" });
  const w = validateConfig({ version: 1, home: { show_title: false } });
  expect(w.ok && w.config.home_screens[0]!.home).toEqual({ title: "curator", show_title: false, language: "auto" });
});

test("paths are checked like on the Rust side", () => {
  for (const ok of ["/", "/a", "/srv/media movies/x.y", "/a/.hidden"]) expect(isNormalizedAbsolutePath(ok)).toBe(true);
  for (const bad of ["", "a", "/a/", "//a", "/a/./b", "/a/..", "/a/../b", "/a\0b"]) expect(isNormalizedAbsolutePath(bad)).toBe(false);
});

test("missing fields take defaults", () => {
  const v = validateConfig({ version: 1 });
  expect(v).toEqual({ ok: true, config: defaultConfig() });
  const w = validateConfig({ version: 1, discs: { enabled: false } });
  expect(w.ok && w.config.home_screens[0]!.discs).toEqual({ ...defaultConfig().home_screens[0]!.discs, enabled: false });
});

// Keep in sync with `rejects_bad_configs` in src/config.rs.
const BAD: unknown[] = [
  { version: 2 },
  {},
  { version: 1, unknown: true },
  { version: 1, discs: { enabeld: true } },
  { version: 1, discs: { audio_languages: ["de"] } },
  { version: 1, discs: { audio_languages: ["DEU"] } },
  { version: 1, discs: { audio_languages: ["deu", "deu"] } },
  { version: 1, discs: { enabled: "yes" } },
  { version: 1, discs: { audio_languages: "deu" } },
  { version: 1, discs: { audio_languages: Array(11).fill(0).map((_, i) => `a${String.fromCharCode(97 + i)}a`) } },
  { version: 1, folders: [{ id: "a", name: "A", path: "/x" }] },
  { version: 1, folders: [{ id: "A", name: "A", path: "/x", video: true }] },
  { version: 1, folders: [{ id: "-a", name: "A", path: "/x", video: true }] },
  { version: 1, folders: [{ id: "a", name: "", path: "/x", video: true }] },
  { version: 1, folders: [{ id: "a", name: " A", path: "/x", video: true }] },
  { version: 1, folders: [{ id: "a", name: "A", path: "x", video: true }] },
  { version: 1, folders: [{ id: "a", name: "A", path: "/x/../etc", video: true }] },
  { version: 1, folders: [{ id: "a", name: "A", path: "/x/", video: true }] },
  { version: 1, folders: [{ id: "a", name: "A", path: "/x//y", video: true }] },
  { version: 1, folders: [{ id: "a", name: "A", path: "/x", video: true, write: true }] },
  { version: 1, folders: [{ id: "a", name: "A", path: "/x", extensions: ["pdf"] }] },
  { version: 1, folders: [{ id: "a", name: "A", path: "/x", video: true }, { id: "a", name: "B", path: "/y", audio: true }] },
  { version: 1, websites: [{ id: "a", name: "A" }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "http://localhost:8787/" }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "https://192.168.1.1/" }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "https://box.local/" }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "file:///etc/passwd" }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "kika.de" }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "https://user:pw@kika.de/" }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "https://kika.de/", allow_subdomains: "yes" }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "https://kika.de/", show_logo: 1 }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "https://kika.de/", extra: 1 }] },
  { version: 1, websites: [{ id: "a", name: "A", url: "https://a.de/" }, { id: "a", name: "B", url: "https://b.de/" }] },
  { version: 1, home: { title: "" } },
  { version: 1, home: { title: " Kino" } },
  { version: 1, home: { title: "a\u0007b" } },
  { version: 1, home: { show_title: "no" } },
  { version: 1, home: { language: "fr" } },
  { version: 1, home: { language: "DE" } },
  { version: 1, home: { language: 1 } },
  { version: 1, home: { logo: "x" } },
  { version: 1, parental: { pin_hash: "1234" } },
  { version: 1, parental: { pin_hash: "$argon2id$x", pin_length: 3 } },
  { version: 1, parental: { pin_hash: "$argon2id$x", pin_length: 9 } },
  { version: 1, parental: { pin_length: 4 } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 0, scope: { kind: "every_day" } }] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 1441, scope: { kind: "every_day" } }] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 60 }] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 60, scope: { kind: "today", date: "2026-13-01" } }] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 60, scope: { kind: "weekdays", days: [] } }] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 60, scope: { kind: "weekdays", days: ["mon", "mon"] } }] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 60, scope: { kind: "weekdays", days: ["monday"] } }] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 60, scope: { kind: "this_week", week_of: "2026-11-03" } }] } },
  { version: 1, parental: { limits: [{ type: "window", until: "7pm", scope: { kind: "every_day" } }] } },
  { version: 1, parental: { limits: [{ type: "window", from: "19:00", until: "18:00", scope: { kind: "every_day" } }] } },
  { version: 1, parental: { limits: [{ type: "window", until: "24:00", scope: { kind: "every_day" } }] } },
  { version: 1, parental: { limits: [{ type: "nap", scope: { kind: "every_day" } }] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 60, scope: { kind: "every_day", x: 1 } }] } },
  { version: 1, screensaver: { mode: "matrix" } },
  { version: 1, screensaver: { mode: "Title" } },
  { version: 1, screensaver: { idle_minutes: 0 } },
  { version: 1, screensaver: { idle_minutes: 241 } },
  { version: 1, screensaver: { idle_minutes: 1.5 } },
  { version: 1, screensaver: { aerial_quality: "8k" } },
  { version: 1, screensaver: { aerial_videos: ["../x"] } },
  { version: 1, screensaver: { aerial_videos: ["a.b"] } },
  { version: 1, screensaver: { aerial_videos: [""] } },
  { version: 1, screensaver: { aerial_videos: ["a", "a"] } },
  { version: 1, screensaver: { aerial_videos: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"] } },
  { version: 1, screensaver: { aerial_videos: [1] } },
  { version: 1, screensaver: { urls: [] } },
  { version: 1, games: { pong: { enabled: "yes" } } },
  { version: 1, games: { pong: { computer_level: "expert" } } },
  { version: 1, games: { pong: { points_to_win: 2 } } },
  { version: 1, games: { pong: { points_to_win: 22 } } },
  { version: 1, games: { pong: { sound: true } } },
  { version: 1, games: { chess: {} } },
  { version: 1, games: [] },
  { version: 1, games: { pong: [] } },
  { version: 1, games: { bricks: { enabled: 1 } } },
  { version: 1, games: { bricks: { lives: 0 } } },
  { version: 1, games: { bricks: { lives: 10 } } },
  { version: 1, games: { bricks: { sound: "on" } } },
  { version: 1, games: { bricks: { speed: 2 } } },
  { version: 1, games: { bricks: [] } },
  { version: 1, programs: [] },
  { version: 1, programs: { pong: { points_to_win: 2 } } },
  { version: 1, programs: { chess: {} } },
  { version: 1, home: [] },
  { version: 1, discs: [true] },
  { version: 1, parental: [] },
  { version: 1, screensaver: ["title"] },
  { version: 1, folders: [["a", "A", "/x", true]] },
  { version: 1, websites: [[]] },
  { version: 1, parental: { limits: [["budget", 60]] } },
  { version: 1, parental: { limits: [{ type: "budget", minutes: 60, scope: ["every_day"] }] } },
  // Version 2: home screens.
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, games: { blocks: { start_level: 10 } } }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, games: { blocks: { enabled: 1 } } }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, games: { blocks: { speed: 1 } } }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, games: { blocks: [] } }] },
  { version: 2 },
  { version: 2, home_screens: [] },
  { version: 2, home_screens: [{ id: "a", name: "A" }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true }, { id: "b", name: "B", default: true }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true }, { id: "a", name: "B" }] },
  { version: 2, home_screens: [{ id: "A", name: "A", default: true }] },
  { version: 2, home_screens: [{ id: "a", name: "", default: true }] },
  { version: 2, home_screens: [{ id: "a", name: "A name that is much too long for it", default: true }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, pin: "1" }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, folders: [{ id: "f", name: "F", path: "/f", video: true }] }, { id: "b", name: "B", folders: [{ id: "f", name: "F", path: "/g", video: true }] }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, websites: [{ id: "w", name: "W", url: "https://a.de/" }] }, { id: "b", name: "B", websites: [{ id: "w", name: "W", url: "https://b.de/" }] }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, limits: [{ type: "budget", minutes: 0, scope: { kind: "every_day" } }] }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, discs: [] }] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true, limits: [["budget"]] }] },
  { version: 2, home_screens: [["a"]] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true }], statistics: { keep_days: 0 } },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true }], statistics: { keep_days: 3651 } },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true }], statistics: { enabled: "yes" } },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true }], statistics: [] },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true }], parental: { limits: [] } },
  { version: 2, home_screens: [{ id: "a", name: "A", default: true }], folders: [] },
  { version: 3, home_screens: [{ id: "a", name: "A", default: true }] },
  "not an object",
  null,
  [],
];

// Wrapped in [ ] because test.each spreads array entries into arguments.
test.each(BAD.map((input) => [input]))("rejects %p", (input) => {
  expect(validateConfig(input).ok).toBe(false);
});

test("the old folders (familynovice) are taken over once", async () => {
  const { mkdtempSync, mkdirSync, writeFileSync, existsSync, readFileSync } = await import("node:fs");
  const { join } = await import("node:path");
  const { migrateOldFolders } = await import("../src/files");
  const home = mkdtempSync("/tmp/curator-migrate-");
  mkdirSync(join(home, ".config", "familynovice"), { recursive: true });
  writeFileSync(join(home, ".config", "familynovice", "config.json"), JSON.stringify({ version: 1, home: { title: "familynovice" } }));
  mkdirSync(join(home, ".cache", "familynovice", "aerial"), { recursive: true });
  mkdirSync(join(home, ".local", "share", "curator"), { recursive: true });
  mkdirSync(join(home, ".local", "share", "familynovice"), { recursive: true });
  await migrateOldFolders(home, {});
  expect(existsSync(join(home, ".config", "familynovice"))).toBe(false);
  expect(JSON.parse(readFileSync(join(home, ".config", "curator", "config.json"), "utf8")).home.title).toBe("curator");
  expect(existsSync(join(home, ".cache", "curator", "aerial"))).toBe(true);
  // Both there: nothing touched.
  expect(existsSync(join(home, ".local", "share", "familynovice"))).toBe(true);
});
