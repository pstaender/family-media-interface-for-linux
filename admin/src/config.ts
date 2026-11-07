// The media center config. Format: docs/config.schema.json.
// Must accept and reject exactly what the Rust side (src/config.rs) does.

export const CONFIG_VERSION = 1;
const MAX_LANGUAGES = 10;
const MAX_FOLDERS = 50;
const MAX_EXTENSIONS = 20;
const MAX_NAME_CHARS = 60;
const MAX_PATH_BYTES = 4096;

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
  /** Extra extensions, lowercase without dot. */
  extensions: string[];
}

export interface Config {
  version: number;
  discs: DiscSettings;
  folders: FolderSettings[];
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
  };
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
  const extra = unknownKey(input, ["id", "name", "path", "video", "audio", "images", "extensions"], "folder");
  if (extra) return extra;
  const { id, name, path } = input;
  if (typeof id !== "string" || !/^[a-z0-9][a-z0-9-]{0,39}$/.test(id)) {
    return `id "${String(id)}" must be lowercase letters, digits and -`;
  }
  if (typeof name !== "string" || [...name].length === 0 || [...name].length > MAX_NAME_CHARS) {
    return `name must have 1 to ${MAX_NAME_CHARS} characters`;
  }
  // Same as Rust: no leading/trailing whitespace, no control characters.
  if (name.trim() !== name || /\p{Cc}/u.test(name)) {
    return "name must not start/end with spaces or contain control characters";
  }
  if (typeof path !== "string" || !isNormalizedAbsolutePath(path)) {
    return `path "${String(path)}" must be an absolute path without '.', '..' or empty parts`;
  }
  const folder: FolderSettings = { id, name, path, video: false, audio: false, images: false, extensions: [] };
  for (const key of ["video", "audio", "images"] as const) {
    if (input[key] === undefined) continue;
    if (typeof input[key] !== "boolean") return `${key}: must be true or false`;
    folder[key] = input[key];
  }
  if (input.extensions !== undefined) {
    const exts = input.extensions;
    if (!Array.isArray(exts)) return "extensions: must be a list";
    if (exts.length > MAX_EXTENSIONS) return `at most ${MAX_EXTENSIONS} extensions`;
    const seen = new Set<string>();
    for (const ext of exts) {
      if (typeof ext !== "string" || !/^[a-z0-9]{1,10}$/.test(ext)) {
        return `extension "${String(ext)}" must be 1-10 lowercase letters/digits, without dot`;
      }
      if (seen.has(ext)) return `extension "${ext}" is listed twice`;
      seen.add(ext);
    }
    folder.extensions = exts as string[];
  }
  if (!(folder.video || folder.audio || folder.images || folder.extensions.length > 0)) {
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
  const top = unknownKey(input, ["version", "discs", "folders"], "config");
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
  return { ok: true, config };
}
