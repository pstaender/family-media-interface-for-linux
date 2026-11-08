import { expect, test } from "bun:test";
import { mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { Aerial, filesToDelete, isVideoUrl, looksLikeMov, parseCatalog, sourceFor } from "../src/aerial";

const SYLVAN = "https://sylvan.apple.com/Videos/comp_A_SDR_2K_AVC.mov";

test("only Apple's video server is accepted", () => {
  expect(isVideoUrl(SYLVAN)).toBe(true);
  expect(isVideoUrl("https://sylvan.apple.com/Aerials/2x/Videos/x_4K_HEVC.MOV")).toBe(true);
  for (const bad of [
    "http://sylvan.apple.com/Videos/a.mov",
    "https://sylvan.apple.com.evil.example/a.mov",
    "https://evil.example/sylvan.apple.com/a.mov",
    "https://user@sylvan.apple.com/a.mov",
    "https://sylvan.apple.com:8443/a.mov",
    "https://sylvan.apple.com/a.mp4",
    "https://sylvan.apple.com/a.mov?x=.mp4".replace(".mov", ".exe"),
    "file:///etc/passwd",
    "not a url",
    42,
  ]) {
    expect(isVideoUrl(bad)).toBe(false);
  }
});

test("the catalog keeps only safe entries", () => {
  const videos = parseCatalog([
    {
      id: "B876B645-3955-420E-99DF-60139E451CF3",
      accessibilityLabel: "China",
      name: "Mountains",
      type: "landscape",
      timeOfDay: "day",
      src: { H2641080p: SYLVAN, H2651080p: "https://evil.example/a.mov", H2654k: "https://sylvan.apple.com/b_4K.mov" },
    },
    { id: "../etc", name: "Bad id", type: "space", src: { H2641080p: SYLVAN } },
    { id: "NO-SOURCES", name: "Elsewhere", type: "space", src: { H2641080p: "https://evil.example/a.mov" } },
    { id: "B876B645-3955-420E-99DF-60139E451CF3", name: "Duplicate", src: { H2641080p: SYLVAN } },
    { id: "ONLY-4K", name: "Ghansali\u0007 from Above", accessibilityLabel: "India", type: "volcano", src: { H2654k240: "https://sylvan.apple.com/c.mov" } },
    "junk",
    null,
  ]);
  expect(videos.map((v) => v.id)).toEqual(["B876B645-3955-420E-99DF-60139E451CF3", "ONLY-4K"]);
  const [mountains, only4k] = videos as [(typeof videos)[0], (typeof videos)[0]];
  expect(mountains.sources).toEqual({ H2641080p: SYLVAN, H2654k: "https://sylvan.apple.com/b_4K.mov" });
  expect(only4k.name).toBe("Ghansali from Above"); // control characters removed
  expect(only4k.type).toBe("other");
  // Quality with fallbacks.
  expect(sourceFor(mountains, "1080p_h264")).toBe(SYLVAN);
  expect(sourceFor(mountains, "1080p_hevc")).toBe(SYLVAN);
  expect(sourceFor(mountains, "4k_hevc")).toBe("https://sylvan.apple.com/b_4K.mov");
  expect(sourceFor(only4k, "1080p_h264")).toBe("https://sylvan.apple.com/c.mov");
  expect(() => parseCatalog({ videos: [] })).toThrow();
});

test("only our own, no longer needed files are deleted", () => {
  const names = [
    "A.1080p_h264.mov", // chosen, current quality
    "B.1080p_h264.mov", // chosen, old quality, new one not there yet: keep
    "B.1080p_hevc.mov.part",
    "C.1080p_h264.mov", // chosen, old quality, new one there: delete
    "C.1080p_hevc.mov",
    "C.4k_hevc.mov.part", // unfinished other quality: delete
    "D.1080p_hevc.mov", // not chosen: delete
    "D.1080p_hevc.mov.part",
    "catalog.json",
    "sizes.json",
    "holiday.mov",
    "../A.1080p_hevc.mov",
    "E.8k.mov",
  ];
  expect(filesToDelete(names, { aerial_videos: ["A", "B", "C"], aerial_quality: "1080p_hevc" }).sort()).toEqual(
    ["C.1080p_h264.mov", "C.4k_hevc.mov.part", "D.1080p_hevc.mov", "D.1080p_hevc.mov.part"].sort(),
  );
  // Nothing chosen: all our video files go, nothing else.
  expect(filesToDelete(names, { aerial_videos: [], aerial_quality: "1080p_h264" })).not.toContain("catalog.json");
  expect(filesToDelete(names, { aerial_videos: [], aerial_quality: "1080p_h264" })).not.toContain("holiday.mov");
});

test("downloads must look like QuickTime files", () => {
  const box = (name: string) => new Uint8Array([0, 0, 0, 20, ...new TextEncoder().encode(name), 0, 0]);
  expect(looksLikeMov(box("ftyp"))).toBe(true);
  expect(looksLikeMov(box("wide"))).toBe(true);
  expect(looksLikeMov(new TextEncoder().encode("<!DOCTYPE html>"))).toBe(false);
  expect(looksLikeMov(new Uint8Array([1, 2, 3]))).toBe(false);
});

test("turning the aerial screensaver off deletes the downloaded videos (and nothing else)", async () => {
  const dir = mkdtempSync("/tmp/fn-aerial-");
  for (const name of ["A.1080p_h264.mov", "B.1080p_h264.mov.part", "catalog.json", "notes.txt"]) {
    writeFileSync(join(dir, name), "x");
  }
  const aerial = new Aerial(dir);
  aerial.sync({ mode: "title", idle_minutes: 10, aerial_quality: "1080p_h264", aerial_videos: ["A"] });
  for (let i = 0; i < 50 && readdirSync(dir).length > 2; i++) await Bun.sleep(10);
  expect(readdirSync(dir).sort()).toEqual(["catalog.json", "notes.txt"]);
});

test("the cached list is read back (and checked again)", async () => {
  const dir = mkdtempSync("/tmp/fn-aerial-cache-");
  const video = { id: "A", name: "Mountains", location: "China", type: "landscape" as const, timeOfDay: "day", sources: { H2641080p: SYLVAN } };
  const evil = { ...video, id: "B", sources: { H2641080p: "https://evil.example/b.mov" } };
  writeFileSync(join(dir, "catalog.json"), JSON.stringify({ fetched_at: Date.now(), videos: [video, evil] }));
  const { videos } = await new Aerial(dir).videos();
  expect(videos).toEqual([video]);
});
