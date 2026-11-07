// The media center config. Format: docs/config.schema.json.
// Must accept and reject exactly what the Rust side (src/config.rs) does.

export const CONFIG_VERSION = 1;
const MAX_LANGUAGES = 10;
const MAX_FOLDERS = 50;
const MAX_NAME_CHARS = 60;
const MAX_PATH_BYTES = 4096;
const MAX_WEBSITES = 50;
const MAX_URL_LENGTH = 2048;

export interface DiscSettings {
  enabled: boolean;
  allow_disc_menu: boolean;
  /** ISO 639-2 codes, most preferred first. */
  audio_languages: string[];
  /** Empty: no subtitles. */
  subtitle_languages: string[];
}

/** A folder shown in the media center. Always accessed read-only. */
export interface FolderSettings {
  id: string;
  name: string;
  /** Absolute, normalized path. */
  path: string;
  video: boolean;
  audio: boolean;
  images: boolean;
}

/** A website tile. Rules: src/sites.rs (pages only on its domain; media from anywhere). */
export interface WebsiteSettings {
  id: string;
  name: string;
  /** Start page. */
  url: string;
  allow_subdomains: boolean;
}

export interface Config {
  version: number;
  discs: DiscSettings;
  folders: FolderSettings[];
  websites: WebsiteSettings[];
}

export function defaultConfig(): Config {
  return {
    version: CONFIG_VERSION,
    discs: {
      enabled: true,
      allow_disc_menu: true,
      audio_languages: [],
      subtitle_languages: [],
    },
    folders: [],
    websites: [],
  };
}

/**
 * The domain a website may show pages from, like `rules_for` in src/sites.rs.
 * Returns an error text for addresses that can't be used.
 */
export function websiteDomain(address: string, allowSubdomains: boolean): { host: string; subdomains: boolean } | string {
  let url: URL;
  try {
    url = new URL(address);
  } catch {
    return "Please enter a full address like https://www.example.com/";
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return "The address must start with https:// or http://";
  if (url.username || url.password) return "The address must not contain a user name or password";
  const host = url.hostname.replace(/\.$/, "").toLowerCase();
  if (host.startsWith("[") || /^\d+\.\d+\.\d+\.\d+$/.test(host)) return "Please use a domain name, not an IP address";
  if (host === "localhost" || host.endsWith(".localhost") || host.endsWith(".local") || !host.includes(".")) {
    return "Addresses in the home network are not allowed";
  }
  return allowSubdomains ? { host: host.replace(/^www\./, ""), subdomains: true } : { host, subdomains: false };
}

function validateWebsite(input: unknown): WebsiteSettings | string {
  if (!isObject(input)) return "must be an object";
  const extra = unknownKey(input, ["id", "name", "url", "allow_subdomains"], "website");
  if (extra) return extra;
  const idName = checkIdAndName(input.id, input.name);
  if (idName) return idName;
  const { url } = input;
  if (typeof url !== "string") return "url: missing";
  if (url.length > MAX_URL_LENGTH) return `url: at most ${MAX_URL_LENGTH} characters`;
  let allow_subdomains = true;
  if (input.allow_subdomains !== undefined) {
    if (typeof input.allow_subdomains !== "boolean") return "allow_subdomains: must be true or false";
    allow_subdomains = input.allow_subdomains;
  }
  const domain = websiteDomain(url, allow_subdomains);
  if (typeof domain === "string") return `url "${url}": ${domain}`;
  return { id: input.id as string, name: input.name as string, url, allow_subdomains };
}

/** Same rules as `validate_id_and_name` in src/config.rs. */
function checkIdAndName(id: unknown, name: unknown): string | null {
  if (typeof id !== "string" || !/^[a-z0-9][a-z0-9-]{0,39}$/.test(id)) {
    return `id "${String(id)}" must be lowercase letters, digits and -`;
  }
  if (typeof name !== "string" || [...name].length === 0 || [...name].length > MAX_NAME_CHARS) {
    return `name must have 1 to ${MAX_NAME_CHARS} characters`;
  }
  if (name.trim() !== name || /\p{Cc}/u.test(name)) {
    return "name must not start/end with spaces or contain control characters";
  }
  return null;
}

/** `/a/b` yes; `a/b`, `/a/../b`, `/a/./b`, `/a//b`, `/a/` no; `/` yes. Same as Rust. */
export function isNormalizedAbsolutePath(path: string): boolean {
  if (path === "/") return true;
  return (
    new TextEncoder().encode(path).length <= MAX_PATH_BYTES &&
    !path.includes("\0") &&
    path.startsWith("/") &&
    path
      .slice(1)
      .split("/")
      .every((part) => part !== "" && part !== "." && part !== "..")
  );
}

function validateFolder(input: unknown): FolderSettings | string {
  if (!isObject(input)) return "must be an object";
  // "extensions" (removed option) is still accepted from older config files, but ignored.
  const extra = unknownKey(input, ["id", "name", "path", "video", "audio", "images", "extensions"], "folder");
  if (extra) return extra;
  const { id, name, path } = input;
  const idName = checkIdAndName(id, name);
  if (idName) return idName;
  if (typeof path !== "string" || !isNormalizedAbsolutePath(path)) {
    return `path "${String(path)}" must be an absolute path without '.', '..' or empty parts`;
  }
  const folder: FolderSettings = { id: id as string, name: name as string, path, video: false, audio: false, images: false };
  for (const key of ["video", "audio", "images"] as const) {
    if (input[key] === undefined) continue;
    if (typeof input[key] !== "boolean") return `${key}: must be true or false`;
    folder[key] = input[key];
  }
  if (!(folder.video || folder.audio || folder.images)) {
    return "choose at least one kind of file to show";
  }
  return folder;
}

export type Validation = { ok: true; config: Config } | { ok: false; error: string };

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function unknownKey(obj: Record<string, unknown>, allowed: string[], where: string): string | null {
  const extra = Object.keys(obj).find((k) => !allowed.includes(k));
  return extra === undefined ? null : `${where}: unknown field "${extra}"`;
}

function languages(value: unknown, field: string): string[] | string {
  if (value === undefined) return [];
  if (!Array.isArray(value)) return `${field}: must be a list`;
  if (value.length > MAX_LANGUAGES) return `${field}: at most ${MAX_LANGUAGES} languages`;
  const seen = new Set<string>();
  for (const lang of value) {
    if (typeof lang !== "string" || !/^[a-z]{3}$/.test(lang)) {
      return `${field}: "${String(lang)}" is not a 3-letter language code like "deu"`;
    }
    if (seen.has(lang)) return `${field}: "${lang}" is listed twice`;
    seen.add(lang);
  }
  return value as string[];
}

/** Validate untrusted input and fill in defaults. */
export function validateConfig(input: unknown): Validation {
  if (!isObject(input)) return { ok: false, error: "config must be an object" };
  const top = unknownKey(input, ["version", "discs", "folders", "websites"], "config");
  if (top) return { ok: false, error: top };
  if (input.version !== CONFIG_VERSION) {
    return {
      ok: false,
      error: `unsupported config version ${String(input.version)} (expected ${CONFIG_VERSION})`,
    };
  }
  const config = defaultConfig();
  if (input.discs !== undefined) {
    const d = input.discs;
    if (!isObject(d)) return { ok: false, error: "discs: must be an object" };
    const extra = unknownKey(
      d,
      ["enabled", "allow_disc_menu", "audio_languages", "subtitle_languages"],
      "discs",
    );
    if (extra) return { ok: false, error: extra };
    for (const key of ["enabled", "allow_disc_menu"] as const) {
      if (d[key] === undefined) continue;
      if (typeof d[key] !== "boolean") return { ok: false, error: `discs.${key}: must be true or false` };
      config.discs[key] = d[key];
    }
    for (const key of ["audio_languages", "subtitle_languages"] as const) {
      const result = languages(d[key], `discs.${key}`);
      if (typeof result === "string") return { ok: false, error: result };
      config.discs[key] = result;
    }
  }
  if (input.folders !== undefined) {
    if (!Array.isArray(input.folders)) return { ok: false, error: "folders: must be a list" };
    if (input.folders.length > MAX_FOLDERS) return { ok: false, error: `folders: at most ${MAX_FOLDERS} folders` };
    const ids = new Set<string>();
    for (const [i, raw] of input.folders.entries()) {
      const folder = validateFolder(raw);
      if (typeof folder === "string") return { ok: false, error: `folders[${i}]: ${folder}` };
      if (ids.has(folder.id)) return { ok: false, error: `folders[${i}]: id "${folder.id}" is used twice` };
      ids.add(folder.id);
      config.folders.push(folder);
    }
  }
  if (input.websites !== undefined) {
    if (!Array.isArray(input.websites)) return { ok: false, error: "websites: must be a list" };
    if (input.websites.length > MAX_WEBSITES) return { ok: false, error: `websites: at most ${MAX_WEBSITES} websites` };
    const ids = new Set<string>();
    for (const [i, raw] of input.websites.entries()) {
      const site = validateWebsite(raw);
      if (typeof site === "string") return { ok: false, error: `websites[${i}]: ${site}` };
      if (ids.has(site.id)) return { ok: false, error: `websites[${i}]: id "${site.id}" is used twice` };
      ids.add(site.id);
      config.websites.push(site);
    }
  }
  return { ok: true, config };
}
