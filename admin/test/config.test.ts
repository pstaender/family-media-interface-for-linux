import { expect, test } from "bun:test";
import example from "../../docs/config.example.json";
import { defaultConfig, isNormalizedAbsolutePath, validateConfig } from "../src/config";

test("the example config is valid", () => {
  const v = validateConfig(example);
  expect(v.ok).toBe(true);
  if (v.ok) expect(v.config.discs.audio_languages).toEqual(["deu", "eng"]);
});

test("the example folders are valid", () => {
  const v = validateConfig(example);
  expect(v.ok && v.config.folders.map((f) => f.id)).toEqual(["movies", "photos", "stories"]);
});

test("the removed extensions option is ignored", () => {
  const v = validateConfig({ version: 1, folders: [{ id: "a", name: "A", path: "/x", video: true, extensions: ["pdf"] }] });
  expect(v.ok && v.config.folders[0]).toEqual({ id: "a", name: "A", path: "/x", video: true, audio: false, images: false });
});

test("paths are checked like on the Rust side", () => {
  for (const ok of ["/", "/a", "/srv/media movies/x.y", "/a/.hidden"]) expect(isNormalizedAbsolutePath(ok)).toBe(true);
  for (const bad of ["", "a", "/a/", "//a", "/a/./b", "/a/..", "/a/../b", "/a\0b"]) expect(isNormalizedAbsolutePath(bad)).toBe(false);
});

test("missing fields take defaults", () => {
  const v = validateConfig({ version: 1 });
  expect(v).toEqual({ ok: true, config: defaultConfig() });
  const w = validateConfig({ version: 1, discs: { enabled: false } });
  expect(w.ok && w.config.discs).toEqual({ ...defaultConfig().discs, enabled: false });
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
  "not an object",
  null,
  [],
];

// Wrapped in [ ] because test.each spreads array entries into arguments.
test.each(BAD.map((input) => [input]))("rejects %p", (input) => {
  expect(validateConfig(input).ok).toBe(false);
});
