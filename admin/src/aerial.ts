// Aerial videos for the media center's screensaver (Apple TV style).
//
// The list of videos comes from the AerialWindows/Aerial project (videos.json, MIT); the
// videos themselves are Apple's and are downloaded from Apple's server (sylvan.apple.com),
// only the ones the parents choose (at most 10), into
// $XDG_CACHE_HOME/curator/aerial/<id>.<quality>.mov. The media center plays these files
// and never fetches anything itself.
//
// Safety:
// - only https://sylvan.apple.com/… addresses from the list are ever used (a changed list
//   can't make us download from elsewhere); redirects must stay on that host,
// - sylvan.apple.com's certificate comes from Apple's own CA (not in the system's list):
//   we trust exactly Apple Root CA for it (apple-root-ca.pem), never "no verification",
// - one download at a time, written to <file>.part and renamed when complete; size checked
//   against Content-Length, free space checked before, the start must look like a QuickTime
//   file; stalled downloads (60 s without data) are aborted,
// - only files named like ours are ever deleted, and only in our own aerial folder.

import { mkdir, readdir, rename, rm, stat, statfs } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { AERIAL_QUALITIES, isAerialId, type AerialQuality, type ScreensaverSettings } from "./config";
import { readJson, writeJsonAtomic } from "./files";

export const CATALOG_URL = "https://raw.githubusercontent.com/AerialWindows/Aerial/master/videos.json";
const VIDEO_HOST = "sylvan.apple.com";
const CATALOG_MAX_BYTES = 2 * 1024 * 1024;
const CATALOG_MAX_AGE_MS = 7 * 24 * 3600 * 1000;
const VIDEO_MAX_BYTES = 3 * 1024 ** 3;
/** Free space that must remain after a download. */
const KEEP_FREE_BYTES = 1024 ** 3;
const STALL_MS = 60_000;
const TIMEOUT_MS = 15_000;
const MAX_REDIRECTS = 3;
const SIZE_REQUESTS_AT_ONCE = 4;
const USER_AGENT = "curator/0.1 (family media center; screensaver videos)";

const APPLE_ROOT_CA = Bun.file(new URL("./apple-root-ca.pem", import.meta.url)).text();

export function defaultAerialDir(): string {
  const cache = process.env.XDG_CACHE_HOME || join(homedir(), ".cache");
  return join(cache, "curator", "aerial");
}

export const VIDEO_TYPES = ["landscape", "cityscape", "underwater", "space", "other"] as const;
export type VideoType = (typeof VIDEO_TYPES)[number];

/** Source keys of videos.json, per quality in order of preference (some videos only have 4K). */
const SOURCE_KEYS: Record<AerialQuality, string[]> = {
  "1080p_h264": ["H2641080p", "H2651080p", "H2654k", "H2654k240"],
  "1080p_hevc": ["H2651080p", "H2641080p", "H2654k", "H2654k240"],
  "4k_hevc": ["H2654k", "H2654k240", "H2651080p", "H2641080p"],
};
const ALL_SOURCE_KEYS = ["H2641080p", "H2651080p", "H2654k", "H2654k240"];

export interface AerialVideo {
  id: string;
  name: string;
  location: string;
  type: VideoType;
  timeOfDay: string;
  /** videos.json source key → https://sylvan.apple.com/… */
  sources: Record<string, string>;
}

/** Only https://sylvan.apple.com/….mov without user, password or port. */
export function isVideoUrl(value: unknown): value is string {
  if (typeof value !== "string" || value.length > 1024) return false;
  try {
    const url = new URL(value);
    return (
      url.protocol === "https:" &&
      url.hostname === VIDEO_HOST &&
      url.port === "" &&
      url.username === "" &&
      url.password === "" &&
      url.pathname.toLowerCase().endsWith(".mov")
    );
  } catch {
    return false;
  }
}

function cleanText(value: unknown, max: number): string {
  if (typeof value !== "string") return "";
  return [...value.replace(/[\p{Cc}\p{Cf}]/gu, "").trim()].slice(0, max).join("");
}

/** Check videos.json; entries with a bad id or no usable source are left out. */
export function parseCatalog(raw: unknown): AerialVideo[] {
  if (!Array.isArray(raw)) throw new Error("the video list is not a list");
  const seen = new Set<string>();
  const videos: AerialVideo[] = [];
  for (const entry of raw.slice(0, 2000)) {
    if (typeof entry !== "object" || entry === null) continue;
    const e = entry as Record<string, unknown>;
    if (!isAerialId(e.id) || seen.has(e.id)) continue;
    const src = typeof e.src === "object" && e.src !== null ? (e.src as Record<string, unknown>) : {};
    const sources: Record<string, string> = {};
    for (const key of ALL_SOURCE_KEYS) if (isVideoUrl(src[key])) sources[key] = src[key] as string;
    if (Object.keys(sources).length === 0) continue;
    const type = (VIDEO_TYPES as readonly unknown[]).includes(e.type) ? (e.type as VideoType) : "other";
    seen.add(e.id);
    videos.push({
      id: e.id,
      name: cleanText(e.name, 80) || "Aerial",
      location: cleanText(e.accessibilityLabel, 80),
      type,
      timeOfDay: cleanText(e.timeOfDay, 20),
      sources,
    });
  }
  return videos;
}

/** The address to download for a quality (falls back when a video lacks it). */
export function sourceFor(video: AerialVideo, quality: AerialQuality): string | null {
  for (const key of SOURCE_KEYS[quality]) if (video.sources[key]) return video.sources[key]!;
  return null;
}

export function fileName(id: string, quality: AerialQuality): string {
  return `${id}.${quality}.mov`;
}

const OUR_FILE = /^([A-Za-z0-9-]{1,64})\.(1080p_h264|1080p_hevc|4k_hevc)\.mov(\.part)?$/;

/**
 * Which files in the aerial folder to delete: ours (by name) for videos no longer chosen,
 * unfinished downloads of another quality, and files of another quality once the chosen
 * quality is there. Other files are never touched.
 */
export function filesToDelete(names: string[], settings: Pick<ScreensaverSettings, "aerial_videos" | "aerial_quality">): string[] {
  const chosen = new Set(settings.aerial_videos);
  const present = new Set(names);
  return names.filter((name) => {
    const m = OUR_FILE.exec(name);
    if (!m) return false;
    const [, id, quality, part] = m as unknown as [string, string, AerialQuality, string | undefined];
    if (!chosen.has(id)) return true;
    if (quality === settings.aerial_quality) return false;
    return part !== undefined || present.has(fileName(id, settings.aerial_quality));
  });
}

/** Does this look like the start of a QuickTime / MP4 file (a known top-level box)? */
export function looksLikeMov(head: Uint8Array): boolean {
  if (head.length < 8) return false;
  const box = new TextDecoder().decode(head.subarray(4, 8));
  return ["ftyp", "moov", "mdat", "wide", "free", "skip", "pnot"].includes(box);
}

/** fetch for Apple's video server: Apple Root CA only, redirects checked hop by hop. */
async function fetchVideoHost(url: string, init: RequestInit): Promise<Response> {
  const ca = await APPLE_ROOT_CA;
  let current = url;
  for (let hop = 0; hop <= MAX_REDIRECTS; hop++) {
    if (!isVideoUrl(current)) throw new Error("address outside Apple's video server");
    const response = await fetch(current, {
      ...init,
      redirect: "manual",
      headers: { "User-Agent": USER_AGENT },
      tls: { ca },
    } as RequestInit);
    if (response.status >= 300 && response.status < 400) {
      const location = response.headers.get("location");
      await response.body?.cancel();
      if (!location) throw new Error(`redirect without address (HTTP ${response.status})`);
      current = new URL(location, current).href;
      continue;
    }
    return response;
  }
  throw new Error("too many redirects");
}

export type DownloadState =
  | { status: "done"; bytes: number }
  | { status: "waiting" }
  | { status: "downloading"; received: number; total: number | null }
  | { status: "error"; error: string };

interface CatalogFile {
  fetched_at: number;
  videos: AerialVideo[];
}

/** The video list, file sizes and downloads. One instance per admin server. */
export class Aerial {
  private catalog: CatalogFile | null = null;
  private catalogError: string | null = null;
  private catalogLoading: Promise<void> | null = null;
  private sizes = new Map<string, number>();
  private sizesLoaded = false;
  private sizing = new Set<string>();
  private states = new Map<string, DownloadState>();
  private settings: ScreensaverSettings | null = null;
  private running: { id: string; abort: AbortController } | null = null;
  private loop: Promise<void> | null = null;
  private again = false;

  constructor(readonly dir: string = defaultAerialDir()) {}

  /** The video list (cached for a week; on errors the cached one stays). */
  async videos(force = false): Promise<{ videos: AerialVideo[]; fetchedAt: number | null; error: string | null }> {
    if (!this.catalog) {
      const cached = (await readJson(join(this.dir, "catalog.json")).catch(() => undefined)) as CatalogFile | undefined;
      if (cached && typeof cached.fetched_at === "number") {
        try {
          // Stored parsed; checked again like a fresh list.
          const raw = Array.isArray(cached.videos)
            ? cached.videos.map((v) => ({ id: v?.id, name: v?.name, accessibilityLabel: v?.location, type: v?.type, timeOfDay: v?.timeOfDay, src: v?.sources }))
            : [];
          this.catalog = { fetched_at: cached.fetched_at, videos: parseCatalog(raw) };
        } catch {
          // broken cache: fetch again
        }
      }
    }
    const stale = !this.catalog || Date.now() - this.catalog.fetched_at > CATALOG_MAX_AGE_MS;
    if (force || stale) {
      this.catalogLoading ??= this.fetchCatalog().finally(() => (this.catalogLoading = null));
      await this.catalogLoading;
    }
    return { videos: this.catalog?.videos ?? [], fetchedAt: this.catalog?.fetched_at ?? null, error: this.catalogError };
  }

  private async fetchCatalog(): Promise<void> {
    try {
      const response = await fetch(CATALOG_URL, {
        headers: { "User-Agent": USER_AGENT },
        signal: AbortSignal.timeout(TIMEOUT_MS),
        redirect: "error",
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const text = await response.text();
      if (text.length > CATALOG_MAX_BYTES) throw new Error("the list is too big");
      const videos = parseCatalog(JSON.parse(text));
      if (videos.length === 0) throw new Error("the list has no usable videos");
      this.catalog = { fetched_at: Date.now(), videos };
      this.catalogError = null;
      await mkdir(this.dir, { recursive: true, mode: 0o700 });
      await writeJsonAtomic(join(this.dir, "catalog.json"), this.catalog);
    } catch (e) {
      this.catalogError = `Could not load the list of aerial videos: ${(e as Error).message}`;
      console.warn(this.catalogError);
    }
  }

  /** Known file sizes (bytes) by address; missing ones are looked up in the background. */
  async sizesFor(urls: string[]): Promise<Map<string, number>> {
    if (!this.sizesLoaded) {
      this.sizesLoaded = true;
      const stored = await readJson(join(this.dir, "sizes.json")).catch(() => undefined);
      if (stored && typeof stored === "object") {
        for (const [url, size] of Object.entries(stored)) {
          if (isVideoUrl(url) && Number.isSafeInteger(size) && (size as number) > 0) this.sizes.set(url, size as number);
        }
      }
    }
    const missing = urls.filter((u) => isVideoUrl(u) && !this.sizes.has(u) && !this.sizing.has(u));
    if (missing.length > 0) void this.lookUpSizes(missing);
    return this.sizes;
  }

  private async lookUpSizes(urls: string[]): Promise<void> {
    for (const u of urls) this.sizing.add(u);
    const queue = [...urls];
    const worker = async () => {
      for (let url = queue.shift(); url; url = queue.shift()) {
        try {
          const r = await fetchVideoHost(url, { method: "HEAD", signal: AbortSignal.timeout(TIMEOUT_MS) });
          const size = Number(r.headers.get("content-length"));
          if (r.ok && Number.isSafeInteger(size) && size > 0) this.sizes.set(url, size);
        } catch {
          // shown as "size unknown"; asked again next time
        } finally {
          this.sizing.delete(url);
        }
      }
    };
    await Promise.all(Array.from({ length: SIZE_REQUESTS_AT_ONCE }, worker));
    await mkdir(this.dir, { recursive: true, mode: 0o700 });
    await writeJsonAtomic(join(this.dir, "sizes.json"), Object.fromEntries(this.sizes)).catch(() => {});
  }

  /** Download state per chosen video. */
  async downloads(): Promise<Record<string, DownloadState>> {
    const out: Record<string, DownloadState> = {};
    const s = this.settings;
    if (!s) return out;
    for (const id of s.aerial_videos) {
      const st = await stat(join(this.dir, fileName(id, s.aerial_quality))).catch(() => null);
      out[id] = st?.isFile() && st.size > 0 ? { status: "done", bytes: st.size } : (this.states.get(id) ?? { status: "waiting" });
    }
    return out;
  }

  async freeBytes(): Promise<number | null> {
    try {
      await mkdir(this.dir, { recursive: true, mode: 0o700 });
      const s = await statfs(this.dir);
      return Number(s.bavail) * Number(s.bsize);
    } catch {
      return null;
    }
  }

  /**
   * Bring the folder in line with the settings: delete what isn't needed, download what's
   * missing (one after the other, in the background). Call after saving and at start.
   */
  sync(settings: ScreensaverSettings): void {
    this.settings = settings;
    if (this.running && !settings.aerial_videos.includes(this.running.id)) this.running.abort.abort();
    // Errors are tried again on each sync.
    for (const [id, state] of this.states) if (state.status === "error") this.states.delete(id);
    this.again = true;
    this.loop ??= this.run().finally(() => (this.loop = null));
  }

  private async run(): Promise<void> {
    while (this.again) {
      this.again = false;
      const settings = this.settings!;
      await mkdir(this.dir, { recursive: true, mode: 0o700 });
      await this.cleanUp(settings);
      if (settings.mode !== "aerial" || settings.aerial_videos.length === 0) continue;
      const { videos } = await this.videos();
      for (const id of settings.aerial_videos) {
        if (this.again) break; // settings changed: start over
        const target = join(this.dir, fileName(id, settings.aerial_quality));
        if (await stat(target).then((s) => s.size > 0, () => false)) continue;
        const video = videos.find((v) => v.id === id);
        const url = video && sourceFor(video, settings.aerial_quality);
        if (!url) {
          this.states.set(id, { status: "error", error: "This video is no longer in the list." });
          continue;
        }
        await this.download(id, url, target);
      }
      await this.cleanUp(settings);
    }
  }

  private async cleanUp(settings: ScreensaverSettings): Promise<void> {
    const names = await readdir(this.dir).catch(() => [] as string[]);
    // Without the aerial screensaver, keep nothing.
    const keep = settings.mode === "aerial" ? settings : { ...settings, aerial_videos: [] };
    for (const name of filesToDelete(names, keep)) {
      if (this.running && name.startsWith(`${this.running.id}.`)) continue;
      await rm(join(this.dir, name), { force: true }).catch((e) => console.warn(`aerial: could not delete ${name}: ${e}`));
    }
  }

  private async download(id: string, url: string, target: string): Promise<void> {
    const part = `${target}.part`;
    const abort = new AbortController();
    this.running = { id, abort };
    let stall: Timer | undefined;
    const kick = () => {
      clearTimeout(stall);
      stall = setTimeout(() => abort.abort(new Error("no data for 60 seconds")), STALL_MS);
    };
    this.states.set(id, { status: "downloading", received: 0, total: null });
    try {
      kick();
      const response = await fetchVideoHost(url, { signal: abort.signal });
      if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`);
      const total = Number(response.headers.get("content-length")) || null;
      if (total !== null && total > VIDEO_MAX_BYTES) throw new Error("the file is too big");
      const free = await this.freeBytes();
      if (total !== null && free !== null && free - total < KEEP_FREE_BYTES) {
        throw new Error(`not enough free space (needs ${formatBytes(total + KEEP_FREE_BYTES)})`);
      }
      const writer = Bun.file(part).writer();
      let received = 0;
      let checked = false;
      try {
        for await (const chunk of response.body as unknown as AsyncIterable<Uint8Array>) {
          kick();
          if (!checked) {
            checked = true;
            if (!looksLikeMov(chunk)) throw new Error("the download is not a video file");
          }
          received += chunk.length;
          if (received > VIDEO_MAX_BYTES) throw new Error("the file is too big");
          writer.write(chunk);
          this.states.set(id, { status: "downloading", received, total });
        }
      } finally {
        await writer.end();
      }
      if (total !== null && received !== total) throw new Error("the download is incomplete");
      await rename(part, target);
      this.states.delete(id);
      console.info(`aerial: downloaded ${id} (${formatBytes(received)})`);
    } catch (e) {
      await rm(part, { force: true }).catch(() => {});
      const reason = abort.signal.aborted && !(abort.signal.reason instanceof Error) ? "stopped" : (e as Error).message;
      if (this.settings?.aerial_videos.includes(id)) {
        this.states.set(id, { status: "error", error: reason });
        console.warn(`aerial: ${id}: ${reason}`);
      } else {
        this.states.delete(id);
      }
    } finally {
      clearTimeout(stall);
      this.running = null;
    }
  }
}

export function formatBytes(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}

export { AERIAL_QUALITIES };
