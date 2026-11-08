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
  /** Show the website's own logo on the tile (fetched by the admin interface, see logos.ts). */
  show_logo: boolean;
}

/** The media center's home screen. */
export interface HomeSettings {
  /** Shown in the top-left corner. */
  title: string;
  show_title: boolean;
  /** Interface language of the media center: "auto" = the computer's language (English if not available). */
  language: Language;
}

export const LANGUAGES = ["auto", "en", "de"] as const;
export type Language = (typeof LANGUAGES)[number];

export const WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;
export type Weekday = (typeof WEEKDAYS)[number];

/** Which days a limit applies to (src/config.rs `Scope`). */
export type Scope =
  | { kind: "today"; date: string }
  | { kind: "weekdays"; days: Weekday[] }
  | { kind: "this_week"; week_of: string }
  | { kind: "every_day" };

/** A time limit (src/config.rs `LimitRule`). */
export type LimitRule =
  | { type: "budget"; minutes: number; scope: Scope }
  | { type: "window"; from?: string; until: string; scope: Scope };

export interface ParentalSettings {
  /** argon2 hash; never sent to the browser (see app.ts). */
  pin_hash?: string;
  /** Number of PIN digits (4-8), only together with pin_hash. Also never sent to the browser. */
  pin_length?: number;
  pin_backoff_doubling: boolean;
  limits: LimitRule[];
}

export const SCREENSAVER_MODES = ["off", "title", "aerial"] as const;
export type ScreensaverMode = (typeof SCREENSAVER_MODES)[number];
export const AERIAL_QUALITIES = ["1080p_h264", "1080p_hevc", "4k_hevc"] as const;
export type AerialQuality = (typeof AERIAL_QUALITIES)[number];
export const MAX_AERIAL_VIDEOS = 10;
const MAX_IDLE_MINUTES = 240;

/** The media center's screensaver (src/config.rs `ScreensaverSettings`). */
export interface ScreensaverSettings {
  mode: ScreensaverMode;
  idle_minutes: number;
  aerial_quality: AerialQuality;
  /** Ids from AerialWindows/Aerial's videos.json; downloaded by aerial.ts. */
  aerial_videos: string[];
}

/** Aerial video ids: safe in file names (same as `is_aerial_id` in src/config.rs). */
export function isAerialId(id: unknown): id is string {
  return typeof id === "string" && /^[A-Za-z0-9-]{1,64}$/.test(id);
}

export interface Config {
  version: number;
  home: HomeSettings;
  parental: ParentalSettings;
  discs: DiscSettings;
  folders: FolderSettings[];
  websites: WebsiteSettings[];
  screensaver: ScreensaverSettings;
}

export function defaultConfig(): Config {
  return {
    version: CONFIG_VERSION,
    home: { title: "familynovice", show_title: true, language: "auto" },
    parental: { pin_backoff_doubling: true, limits: [] },
    discs: {
      enabled: true,
      allow_disc_menu: true,
      audio_languages: [],
      subtitle_languages: [],
    },
    folders: [],
    websites: [],
    screensaver: { mode: "title", idle_minutes: 10, aerial_quality: "1080p_h264", aerial_videos: [] },
  };
}

function validateScreensaver(input: unknown): ScreensaverSettings | string {
  if (!isObject(input)) return "screensaver: must be an object";
  const extra = unknownKey(input, ["mode", "idle_minutes", "aerial_quality", "aerial_videos"], "screensaver");
  if (extra) return extra;
  const s = defaultConfig().screensaver;
  if (input.mode !== undefined) {
    if (!(SCREENSAVER_MODES as readonly unknown[]).includes(input.mode)) {
      return `screensaver.mode must be one of ${SCREENSAVER_MODES.join(", ")}`;
    }
    s.mode = input.mode as ScreensaverMode;
  }
  if (input.idle_minutes !== undefined) {
    const m = input.idle_minutes;
    if (typeof m !== "number" || !Number.isInteger(m) || m < 1 || m > MAX_IDLE_MINUTES) {
      return `screensaver.idle_minutes must be 1 to ${MAX_IDLE_MINUTES}`;
    }
    s.idle_minutes = m;
  }
  if (input.aerial_quality !== undefined) {
    if (!(AERIAL_QUALITIES as readonly unknown[]).includes(input.aerial_quality)) {
      return `screensaver.aerial_quality must be one of ${AERIAL_QUALITIES.join(", ")}`;
    }
    s.aerial_quality = input.aerial_quality as AerialQuality;
  }
  if (input.aerial_videos !== undefined) {
    const list = input.aerial_videos;
    if (!Array.isArray(list)) return "screensaver.aerial_videos: must be a list";
    if (list.length > MAX_AERIAL_VIDEOS) return `screensaver.aerial_videos: at most ${MAX_AERIAL_VIDEOS} videos`;
    for (const [i, id] of list.entries()) {
      if (!isAerialId(id)) return `screensaver.aerial_videos: "${String(id)}" must be letters, digits and -`;
      if (list.slice(0, i).includes(id)) return `screensaver.aerial_videos: "${id}" is listed twice`;
    }
    s.aerial_videos = list as string[];
  }
  return s;
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
  const extra = unknownKey(input, ["id", "name", "url", "allow_subdomains", "show_logo"], "website");
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
  let show_logo = true;
  if (input.show_logo !== undefined) {
    if (typeof input.show_logo !== "boolean") return "show_logo: must be true or false";
    show_logo = input.show_logo;
  }
  const domain = websiteDomain(url, allow_subdomains);
  if (typeof domain === "string") return `url "${url}": ${domain}`;
  return { id: input.id as string, name: input.name as string, url, allow_subdomains, show_logo };
}

const MAX_LIMITS = 50;

/** `YYYY-MM-DD` → a valid date (UTC noon, only used for weekday/date checks). */
export function parseDate(text: unknown): Date | null {
  if (typeof text !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(text)) return null;
  const date = new Date(`${text}T12:00:00Z`);
  return !Number.isNaN(date.getTime()) && date.toISOString().slice(0, 10) === text ? date : null;
}

/** `HH:MM` → minutes since midnight. */
export function parseTime(text: unknown): number | null {
  if (typeof text !== "string" || !/^\d{2}:\d{2}$/.test(text)) return null;
  const [h, m] = text.split(":").map(Number) as [number, number];
  return h < 24 && m < 60 ? h * 60 + m : null;
}

function validateScope(input: unknown): Scope | string {
  if (!isObject(input)) return "scope: must be an object";
  switch (input.kind) {
    case "today": {
      const extra = unknownKey(input, ["kind", "date"], "scope");
      if (extra) return extra;
      if (!parseDate(input.date)) return `date "${String(input.date)}" must be YYYY-MM-DD`;
      return { kind: "today", date: input.date as string };
    }
    case "weekdays": {
      const extra = unknownKey(input, ["kind", "days"], "scope");
      if (extra) return extra;
      const days = input.days;
      if (!Array.isArray(days) || days.length === 0) return "choose at least one weekday";
      const seen = new Set<string>();
      for (const d of days) {
        if (!WEEKDAYS.includes(d)) return `"${String(d)}" is not a weekday (mon … sun)`;
        if (seen.has(d)) return `"${d}" is listed twice`;
        seen.add(d);
      }
      return { kind: "weekdays", days: days as Weekday[] };
    }
    case "this_week": {
      const extra = unknownKey(input, ["kind", "week_of"], "scope");
      if (extra) return extra;
      const monday = parseDate(input.week_of);
      if (!monday) return `week_of "${String(input.week_of)}" must be YYYY-MM-DD`;
      if (monday.getUTCDay() !== 1) return `week_of "${String(input.week_of)}" must be a Monday`;
      return { kind: "this_week", week_of: input.week_of as string };
    }
    case "every_day": {
      const extra = unknownKey(input, ["kind"], "scope");
      return extra ?? { kind: "every_day" };
    }
    default:
      return `scope: unknown kind "${String(input.kind)}"`;
  }
}

function validateLimit(input: unknown): LimitRule | string {
  if (!isObject(input)) return "must be an object";
  if (input.type === "budget") {
    const extra = unknownKey(input, ["type", "minutes", "scope"], "limit");
    if (extra) return extra;
    const scope = validateScope(input.scope);
    if (typeof scope === "string") return scope;
    const max = scope.kind === "this_week" ? 7 * 24 * 60 : 24 * 60;
    const minutes = input.minutes;
    if (typeof minutes !== "number" || !Number.isInteger(minutes) || minutes < 1 || minutes > max) {
      return `minutes must be 1 to ${max}`;
    }
    return { type: "budget", minutes, scope };
  }
  if (input.type === "window") {
    const extra = unknownKey(input, ["type", "from", "until", "scope"], "limit");
    if (extra) return extra;
    const until = parseTime(input.until);
    if (until === null) return `until "${String(input.until)}" must be HH:MM`;
    let from: number | null = null;
    if (input.from !== undefined) {
      from = parseTime(input.from);
      if (from === null) return `from "${String(input.from)}" must be HH:MM`;
      if (from >= until) return "from must be earlier than until";
    }
    const scope = validateScope(input.scope);
    if (typeof scope === "string") return scope;
    const rule: LimitRule = { type: "window", until: input.until as string, scope };
    if (input.from !== undefined) rule.from = input.from as string;
    return rule;
  }
  return `unknown type "${String(input.type)}"`;
}

function validateParental(input: unknown): ParentalSettings | string {
  if (!isObject(input)) return "parental: must be an object";
  const extra = unknownKey(input, ["pin_hash", "pin_length", "pin_backoff_doubling", "limits"], "parental");
  if (extra) return extra;
  const parental: ParentalSettings = { pin_backoff_doubling: true, limits: [] };
  if (input.pin_hash !== undefined) {
    const h = input.pin_hash;
    if (typeof h !== "string" || !h.startsWith("$argon2") || h.length > 512) {
      return "parental.pin_hash must be an argon2 hash";
    }
    parental.pin_hash = h;
  }
  if (input.pin_length !== undefined) {
    const len = input.pin_length;
    if (typeof len !== "number" || !Number.isInteger(len) || len < 4 || len > 8) return "parental.pin_length must be 4 to 8";
    if (parental.pin_hash === undefined) return "parental.pin_length needs a pin_hash";
    parental.pin_length = len;
  }
  if (input.pin_backoff_doubling !== undefined) {
    if (typeof input.pin_backoff_doubling !== "boolean") return "parental.pin_backoff_doubling: must be true or false";
    parental.pin_backoff_doubling = input.pin_backoff_doubling;
  }
  if (input.limits !== undefined) {
    if (!Array.isArray(input.limits)) return "parental.limits: must be a list";
    if (input.limits.length > MAX_LIMITS) return `parental.limits: at most ${MAX_LIMITS} rules`;
    for (const [i, raw] of input.limits.entries()) {
      const rule = validateLimit(raw);
      if (typeof rule === "string") return `parental.limits[${i}]: ${rule}`;
      parental.limits.push(rule);
    }
  }
  return parental;
}

/** Same rules as `validate_id_and_name` in src/config.rs. */
function checkIdAndName(id: unknown, name: unknown): string | null {
  if (typeof id !== "string" || !/^[a-z0-9][a-z0-9-]{0,39}$/.test(id)) {
    return `id "${String(id)}" must be lowercase letters, digits and -`;
  }
  const problem = checkText(name);
  return problem ? `name ${problem}` : null;
}

/** Names and titles, like `check_text` in src/config.rs. */
function checkText(text: unknown): string | null {
  if (typeof text !== "string" || [...text].length === 0 || [...text].length > MAX_NAME_CHARS) {
    return `must have 1 to ${MAX_NAME_CHARS} characters`;
  }
  if (text.trim() !== text || /\p{Cc}/u.test(text)) {
    return "must not start/end with spaces or contain control characters";
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
  const top = unknownKey(input, ["version", "home", "parental", "discs", "folders", "websites", "screensaver"], "config");
  if (top) return { ok: false, error: top };
  if (input.version !== CONFIG_VERSION) {
    return {
      ok: false,
      error: `unsupported config version ${String(input.version)} (expected ${CONFIG_VERSION})`,
    };
  }
  const config = defaultConfig();
  if (input.home !== undefined) {
    const h = input.home;
    if (!isObject(h)) return { ok: false, error: "home: must be an object" };
    const extra = unknownKey(h, ["title", "show_title", "language"], "home");
    if (extra) return { ok: false, error: extra };
    if (h.title !== undefined) {
      const problem = checkText(h.title);
      if (problem) return { ok: false, error: `home.title: ${problem}` };
      config.home.title = h.title as string;
    }
    if (h.show_title !== undefined) {
      if (typeof h.show_title !== "boolean") return { ok: false, error: "home.show_title: must be true or false" };
      config.home.show_title = h.show_title;
    }
    if (h.language !== undefined) {
      if (!(LANGUAGES as readonly unknown[]).includes(h.language)) {
        return { ok: false, error: `home.language must be one of ${LANGUAGES.join(", ")}` };
      }
      config.home.language = h.language as Language;
    }
  }
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
  if (input.parental !== undefined) {
    const parental = validateParental(input.parental);
    if (typeof parental === "string") return { ok: false, error: parental };
    config.parental = parental;
  }
  if (input.screensaver !== undefined) {
    const s = validateScreensaver(input.screensaver);
    if (typeof s === "string") return { ok: false, error: s };
    config.screensaver = s;
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
