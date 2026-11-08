// Website logos for the media center's tiles.
//
// When the config is saved, the admin interface looks at each website's start page (only
// for websites with `show_logo`) and picks the largest usable picture:
//   1. icons with a known size ≥ 96 px (web app manifest, apple-touch-icon, <link rel=icon
//      sizes=…>), largest first,
//   2. the page's preview image (og:image / twitter:image),
//   3. smaller icons, then /favicon.ico.
// SVG is skipped (the media center can't decode it). The picture is stored in
// $XDG_CACHE_HOME/curator/logos/<id>.img plus <id>.json (where it came from); the media
// center notices the new file and shows it. Nothing is fetched at runtime by the media center.
//
// Safety: only http(s) to public domain names (same rules as website start pages; names that
// resolve to home-network addresses are refused too), redirects checked hop by hop, size and
// time limits, and the content must really be a picture (magic bytes).

import { lookup } from "node:dns/promises";
import { isIP } from "node:net";
import { homedir } from "node:os";
import { join } from "node:path";
import { mkdir, readFile, readdir, rename, rm, stat } from "node:fs/promises";
import { websiteDomain, type WebsiteSettings } from "./config";

const PAGE_MAX_BYTES = 2 * 1024 * 1024;
const MANIFEST_MAX_BYTES = 256 * 1024;
const IMAGE_MAX_BYTES = 4 * 1024 * 1024;
const TIMEOUT_MS = 10_000;
const MAX_REDIRECTS = 4;
const USER_AGENT = "curator/0.1 (family media center; fetches the site logo)";
/** Icons at least this big count as "large"; smaller ones come after the preview image. */
const LARGE_ICON = 96;

export function defaultLogoDir(): string {
  const cache = process.env.XDG_CACHE_HOME || join(homedir(), ".cache");
  return join(cache, "curator", "logos");
}

export interface Candidate {
  url: string;
  /** Largest edge in px if known (from `sizes`), else 0. */
  size: number;
  kind: "icon" | "preview" | "favicon";
}

/** Order candidates: large icons (largest first), preview image, small icons, favicon. */
export function rankCandidates(candidates: Candidate[]): Candidate[] {
  const group = (c: Candidate) =>
    c.kind === "favicon" ? 3 : c.kind === "preview" ? 1 : c.size >= LARGE_ICON ? 0 : 2;
  const seen = new Set<string>();
  return candidates
    .filter((c) => !isSvg(c.url))
    .sort((a, b) => group(a) - group(b) || b.size - a.size)
    .filter((c) => !seen.has(c.url) && Boolean(seen.add(c.url)));
}

function isSvg(url: string): boolean {
  return /\.svgz?($|[?#])/i.test(url);
}

/** Largest edge from a `sizes` attribute like "192x192" or "16x16 32x32" ("any" = SVG). */
export function parseSizes(sizes: string | null | undefined): number {
  let best = 0;
  for (const m of (sizes ?? "").matchAll(/(\d+)\s*x\s*(\d+)/gi)) {
    best = Math.max(best, Math.min(Number(m[1]), Number(m[2])));
  }
  return best;
}

function resolve(href: string | null | undefined, base: string): string | null {
  if (!href) return null;
  try {
    const url = new URL(href.trim(), base);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}

/** Icon / preview candidates and the web app manifest link from a page's HTML. */
export async function scanPage(html: string, pageUrl: string): Promise<{ candidates: Candidate[]; manifest: string | null }> {
  const candidates: Candidate[] = [];
  let manifest: string | null = null;
  let base = pageUrl;
  const rewriter = new HTMLRewriter()
    .on("base[href]", {
      element(el) {
        base = resolve(el.getAttribute("href"), pageUrl) ?? base;
      },
    })
    .on("link[rel][href]", {
      element(el) {
        const rels = (el.getAttribute("rel") ?? "").toLowerCase().split(/\s+/);
        const url = resolve(el.getAttribute("href"), base);
        if (!url) return;
        if (rels.includes("manifest")) manifest ??= url;
        if (el.getAttribute("type")?.includes("svg")) return;
        if (rels.includes("apple-touch-icon") || rels.includes("apple-touch-icon-precomposed")) {
          // Without sizes, apple-touch-icons are usually 180 × 180.
          candidates.push({ url, size: parseSizes(el.getAttribute("sizes")) || 180, kind: "icon" });
        } else if (rels.includes("icon")) {
          candidates.push({ url, size: parseSizes(el.getAttribute("sizes")), kind: "icon" });
        }
      },
    })
    .on("meta[content]", {
      element(el) {
        const key = (el.getAttribute("property") ?? el.getAttribute("name") ?? "").toLowerCase();
        if (["og:image", "og:image:url", "og:image:secure_url", "twitter:image"].includes(key)) {
          const url = resolve(el.getAttribute("content"), base);
          if (url) candidates.push({ url, size: 0, kind: "preview" });
        }
      },
    });
  await rewriter.transform(new Response(html)).text();
  candidates.push({ url: new URL("/favicon.ico", pageUrl).href, size: 0, kind: "favicon" });
  return { candidates, manifest };
}

/** Icons from a web app manifest (`icons: [{src, sizes, type, purpose}]`). */
export function manifestIcons(manifest: unknown, manifestUrl: string): Candidate[] {
  if (typeof manifest !== "object" || manifest === null) return [];
  const icons = (manifest as { icons?: unknown }).icons;
  if (!Array.isArray(icons)) return [];
  const out: Candidate[] = [];
  for (const icon of icons) {
    if (typeof icon !== "object" || icon === null) continue;
    const { src, sizes, type, purpose } = icon as Record<string, unknown>;
    if (typeof type === "string" && type.includes("svg")) continue;
    // Monochrome icons are single-colour silhouettes; not what we want on a tile.
    if (typeof purpose === "string" && !purpose.split(/\s+/).some((p) => p === "any" || p === "maskable")) continue;
    const url = resolve(typeof src === "string" ? src : null, manifestUrl);
    if (url) out.push({ url, size: parseSizes(typeof sizes === "string" ? sizes : null), kind: "icon" });
  }
  return out;
}

/** The picture format by magic bytes, or null if it's not a picture the media center reads. */
export function imageType(bytes: Uint8Array): string | null {
  const starts = (...b: number[]) => b.every((v, i) => bytes[i] === v);
  if (starts(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a)) return "image/png";
  if (starts(0xff, 0xd8, 0xff)) return "image/jpeg";
  if (starts(0x47, 0x49, 0x46, 0x38)) return "image/gif";
  if (starts(0x52, 0x49, 0x46, 0x46) && bytes[8] === 0x57 && bytes[9] === 0x45 && bytes[10] === 0x42 && bytes[11] === 0x50) return "image/webp";
  if (starts(0x00, 0x00, 0x01, 0x00)) return "image/x-icon";
  if (starts(0x42, 0x4d)) return "image/bmp";
  return null;
}

/** Is this an address in the home network / loopback / link-local (or otherwise not public)? */
export function isPrivateAddress(ip: string): boolean {
  const v4 = ip.startsWith("::ffff:") ? ip.slice(7) : ip;
  if (isIP(v4) === 4) {
    const [a, b] = v4.split(".").map(Number) as [number, number];
    return a === 0 || a === 10 || a === 127 || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31) ||
      (a === 192 && b === 168) || (a === 100 && b >= 64 && b <= 127) || a >= 224;
  }
  const v6 = ip.toLowerCase();
  return v6 === "::" || v6 === "::1" || v6.startsWith("fe8") || v6.startsWith("fe9") || v6.startsWith("fea") ||
    v6.startsWith("feb") || v6.startsWith("fc") || v6.startsWith("fd") || v6.startsWith("ff");
}

export type Resolver = (host: string) => Promise<string[]>;
const dnsResolver: Resolver = async (host) => (await lookup(host, { all: true })).map((a) => a.address);

/** Only public web addresses (like website start pages), checked again after DNS. */
async function checkUrl(url: string, resolveHost: Resolver): Promise<string | null> {
  const domain = websiteDomain(url, false);
  if (typeof domain === "string") return domain;
  const addresses = await resolveHost(domain.host).catch(() => []);
  if (addresses.length === 0) return `${domain.host} not found`;
  if (addresses.some(isPrivateAddress)) return `${domain.host} is in the home network`;
  return null;
}

/** GET with checked redirects, a time limit and a size limit. */
async function fetchLimited(url: string, maxBytes: number, resolveHost: Resolver, accept: string): Promise<{ url: string; bytes: Uint8Array; type: string }> {
  let current = url;
  const signal = AbortSignal.timeout(TIMEOUT_MS);
  for (let hop = 0; hop <= MAX_REDIRECTS; hop++) {
    const problem = await checkUrl(current, resolveHost);
    if (problem) throw new Error(problem);
    const res = await fetch(current, { redirect: "manual", signal, headers: { "User-Agent": USER_AGENT, Accept: accept } });
    if (res.status >= 300 && res.status < 400 && res.headers.get("location")) {
      current = new URL(res.headers.get("location")!, current).href;
      await res.body?.cancel();
      continue;
    }
    if (!res.ok || !res.body) throw new Error(`${current}: HTTP ${res.status}`);
    if (Number(res.headers.get("content-length") ?? 0) > maxBytes) throw new Error(`${current}: too big`);
    const chunks: Uint8Array[] = [];
    let total = 0;
    for await (const chunk of res.body) {
      total += chunk.length;
      if (total > maxBytes) throw new Error(`${current}: too big`);
      chunks.push(chunk);
    }
    return { url: current, bytes: Buffer.concat(chunks), type: res.headers.get("content-type") ?? "" };
  }
  throw new Error(`${url}: too many redirects`);
}

export interface LogoMeta {
  /** The website's start page the logo belongs to. */
  url: string;
  /** Where the picture came from. */
  source: string;
  fetched_at: string;
}

/** Find and download the logo of one website (doesn't write anything). */
export async function findLogo(pageUrl: string, resolveHost: Resolver = dnsResolver): Promise<{ source: string; bytes: Uint8Array }> {
  const page = await fetchLimited(pageUrl, PAGE_MAX_BYTES, resolveHost, "text/html,*/*;q=0.5");
  const { candidates, manifest } = await scanPage(new TextDecoder().decode(page.bytes), page.url);
  if (manifest) {
    try {
      const m = await fetchLimited(manifest, MANIFEST_MAX_BYTES, resolveHost, "application/manifest+json,application/json");
      candidates.push(...manifestIcons(JSON.parse(new TextDecoder().decode(m.bytes)), m.url));
    } catch (e) {
      console.warn(`logo: manifest ${manifest}: ${(e as Error).message}`);
    }
  }
  const errors: string[] = [];
  for (const candidate of rankCandidates(candidates)) {
    try {
      const image = await fetchLimited(candidate.url, IMAGE_MAX_BYTES, resolveHost, "image/png,image/jpeg,image/webp,image/*;q=0.8");
      if (!imageType(image.bytes)) throw new Error(`${candidate.url}: not a supported picture`);
      return { source: image.url, bytes: image.bytes };
    } catch (e) {
      errors.push((e as Error).message);
    }
  }
  throw new Error(errors.length ? `no usable picture (${errors.slice(0, 3).join("; ")})` : "no picture found");
}

const logoFile = (dir: string, id: string) => join(dir, `${id}.img`);
const metaFile = (dir: string, id: string) => join(dir, `${id}.json`);

export async function readLogo(dir: string, id: string): Promise<{ bytes: Buffer; type: string; meta: LogoMeta | null } | null> {
  try {
    const bytes = await readFile(logoFile(dir, id));
    const meta = await readFile(metaFile(dir, id), "utf8").then((t) => JSON.parse(t) as LogoMeta, () => null);
    return { bytes, type: imageType(bytes) ?? "application/octet-stream", meta };
  } catch {
    return null;
  }
}

/** Fetch one website's logo and store it (atomically: the media center never sees half a file). */
export async function updateLogo(dir: string, site: WebsiteSettings, resolveHost?: Resolver): Promise<LogoMeta> {
  const { source, bytes } = await findLogo(site.url, resolveHost);
  await mkdir(dir, { recursive: true, mode: 0o700 });
  const meta: LogoMeta = { url: site.url, source, fetched_at: new Date().toISOString() };
  const tmp = `${logoFile(dir, site.id)}.${process.pid}.tmp`;
  await Bun.write(tmp, bytes);
  await Bun.write(metaFile(dir, site.id), `${JSON.stringify(meta, null, 2)}\n`);
  await rename(tmp, logoFile(dir, site.id));
  return meta;
}

/**
 * Bring the logo cache in line with the config: fetch missing logos (or all with `force`),
 * remove logos of websites that are gone or don't want one. Returns true if anything changed.
 * Errors are logged, never thrown: a missing logo just means the generic icon.
 */
export async function refreshLogos(dir: string, websites: WebsiteSettings[], opts: { force?: boolean; resolveHost?: Resolver } = {}): Promise<boolean> {
  let changed = false;
  const wanted = new Set(websites.filter((w) => w.show_logo).map((w) => w.id));
  // Remove logos nobody wants any more (only our own *.img / *.json files).
  try {
    for (const name of await readdir(dir)) {
      const m = /^([a-z0-9][a-z0-9-]{0,39})\.(img|json)$/.exec(name);
      if (m && !wanted.has(m[1]!)) {
        await rm(join(dir, name), { force: true });
        changed = true;
      }
    }
  } catch {
    // no logo directory yet
  }
  for (const site of websites) {
    if (!site.show_logo) continue;
    const existing = await readLogo(dir, site.id);
    const fresh = existing?.meta?.url === site.url && (await stat(logoFile(dir, site.id)).then(() => true, () => false));
    if (fresh && !opts.force) continue;
    try {
      const meta = await updateLogo(dir, site, opts.resolveHost);
      console.log(`logo for ${site.id}: ${meta.source}`);
      changed = true;
    } catch (e) {
      console.warn(`logo for ${site.id}: ${(e as Error).message}`);
    }
  }
  return changed;
}
