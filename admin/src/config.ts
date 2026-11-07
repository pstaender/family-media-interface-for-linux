// The media center config. Format: docs/config.schema.json.
// Must accept and reject exactly what the Rust side (src/config.rs) does.

export const CONFIG_VERSION = 1;
const MAX_LANGUAGES = 10;

export interface DiscSettings {
  enabled: boolean;
  allow_disc_menu: boolean;
  /** ISO 639-2 codes, most preferred first. */
  audio_languages: string[];
  /** Empty: no subtitles. */
  subtitle_languages: string[];
}

export interface Config {
  version: number;
  discs: DiscSettings;
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
  };
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
  const top = unknownKey(input, ["version", "discs"], "config");
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
  return { ok: true, config };
}
