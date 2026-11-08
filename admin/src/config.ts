// The media center config. Format: docs/config.schema.json.
// Must accept and reject exactly what the Rust side (src/config.rs) does.

/** 2: home screens, each with its own items and time limits; version 1 is still read. */
export const CONFIG_VERSION = 2;
export const MAX_HOME_SCREENS = 10;
export const MAX_SCREEN_NAME_CHARS = 30;
/** The home screen a version-1 config becomes (same as `V1_SCREEN_ID` in src/config.rs). */
export const V1_SCREEN_ID = "home";
export const MAX_KEEP_DAYS = 3650;
const MAX_LANGUAGES = 10;
const MAX_FOLDERS = 50;
const MAX_NAME_CHARS = 60;
const MAX_PATH_BYTES = 4096;
const MAX_WEBSITES = 50;
const MAX_PROGRAMS = 50;
const MAX_EMULATORS = 50;
const MAX_DRIVES = 8;
const MAX_AUTOSTART_CHARS = 260;
const MAX_STARTUP_SCRIPT_CHARS = 4000;
const MAX_PROGRAM_ARGS = 64;
const MAX_COMMAND_CHARS = 1024;
const MAX_ARG_CHARS = 4096;
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
  remember_recent: boolean;
}

/** A website tile. Rules: src/sites.rs (pages only on its domain; media from anywhere). */
/** Z: is DOSBox's own drive. */
export const DOS_DRIVE_LETTERS = [..."ABCDEFGHIJKLMNOPQRSTUVWXY"];
export const DOS_IMAGE_EXTENSIONS = ["img", "ima", "iso", "cue"];
/** A .cue file needs its data file next to it, so it can't be uploaded alone. */
export const DOS_UPLOAD_EXTENSIONS = ["img", "ima", "iso"];
export const MAX_ALLOWED_HOSTS = 32;
export const HOST_NAME = /^(?=.{1,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$/;

/** The admin panel's extra host names, from lines of text (both the browser and the server check). */
export function checkAllowedHosts(lines: unknown): { ok: true; hosts: string[] } | { ok: false; error: string } {
  if (!Array.isArray(lines) || !lines.every((l) => typeof l === "string")) return { ok: false, error: "Expected a list of host names." };
  const hosts: string[] = [];
  for (const [i, raw] of (lines as string[]).entries()) {
    const name = raw.trim().toLowerCase();
    if (name === "") continue;
    if (!HOST_NAME.test(name)) return { ok: false, error: `Line ${i + 1}: "${raw.trim()}" is not a host name (just the name, without http:// or a port).` };
    if (!hosts.includes(name)) hosts.push(name);
  }
  if (hosts.length > MAX_ALLOWED_HOSTS) return { ok: false, error: `At most ${MAX_ALLOWED_HOSTS} host names.` };
  return { ok: true, hosts };
}
export const SNES_ROM_EXTENSIONS = ["sfc", "smc", "swc", "fig", "zip"];
export const SNES_FILTERS = ["none", "crt_easymode", "zfast_crt", "crt_geom", "crt_lottes", "crt_royale"] as const;
export type SnesFilter = (typeof SNES_FILTERS)[number];
export const SNES_ASPECTS = ["4_3", "8_7", "fill"] as const;
export type SnesAspect = (typeof SNES_ASPECTS)[number];

export interface DriveMapping {
  letter: string;
  path: string;
}

export interface DosOptions {
  drives: DriveMapping[];
  autostart: string;
  startup_script: string;
  correct_aspect: boolean;
  keep_saves: boolean;
}

export interface SnesOptions {
  rom: string;
  filter: SnesFilter;
  aspect: SnesAspect;
  integer_scale: boolean;
  resume: boolean;
}

export interface EmulatorSettings {
  id: string;
  name: string;
  system: "dos" | "snes";
  dos: DosOptions;
  snes: SnesOptions;
}

export function newEmulator(id: string): EmulatorSettings {
  return {
    id,
    name: "",
    system: "dos",
    dos: { drives: [], autostart: "", startup_script: "", correct_aspect: true, keep_saves: true },
    snes: { rom: "", filter: "crt_easymode", aspect: "4_3", integer_scale: false, resume: true },
  };
}

export const PROGRAM_ICONS = ["gamepad", "joystick", "computer", "handheld", "floppy", "app"] as const;
export type ProgramIcon = (typeof PROGRAM_ICONS)[number];

/** A program in the "Programs" row: started as registered, never through a shell. */
export interface ProgramSettings {
  id: string;
  name: string;
  /** An absolute path, or a program name looked up in PATH. */
  command: string;
  args: string[];
  working_dir?: string;
  icon: ProgramIcon;
}

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

export const PONG_LEVELS = ["easy", "normal", "hard"] as const;
export type PongLevel = (typeof PONG_LEVELS)[number];

/** The "Games" row (src/config.rs `GameSettings`): built-in games. */
export interface GameSettings {
  pong: { enabled: boolean; computer_level: PongLevel; points_to_win: number };
  bricks: { enabled: boolean; lives: number; sound: boolean };
  blocks: { enabled: boolean; start_level: number; sound: boolean };
  dodger: { enabled: boolean; sound: boolean; music: boolean };
}

function validateGames(input: unknown, where: string): GameSettings | string {
  if (!isObject(input)) return `${where}: must be an object`;
  const extra = unknownKey(input, ["pong", "bricks", "blocks", "dodger"], where);
  if (extra) return extra;
  const games = newHomeScreen("x", "x").games;
  if (input.pong !== undefined) {
    const p = input.pong;
    if (!isObject(p)) return `${where}.pong: must be an object`;
    const extraPong = unknownKey(p, ["enabled", "computer_level", "points_to_win"], `${where}.pong`);
    if (extraPong) return extraPong;
    if (p.enabled !== undefined) {
      if (typeof p.enabled !== "boolean") return `${where}.pong.enabled: must be true or false`;
      games.pong.enabled = p.enabled;
    }
    if (p.computer_level !== undefined) {
      if (!(PONG_LEVELS as readonly unknown[]).includes(p.computer_level)) {
        return `${where}.pong.computer_level must be one of ${PONG_LEVELS.join(", ")}`;
      }
      games.pong.computer_level = p.computer_level as PongLevel;
    }
    if (p.points_to_win !== undefined) {
      const n = p.points_to_win;
      if (typeof n !== "number" || !Number.isInteger(n) || n < 3 || n > 21) return `${where}.pong.points_to_win must be 3 to 21`;
      games.pong.points_to_win = n;
    }
  }
  if (input.bricks !== undefined) {
    const b = input.bricks;
    if (!isObject(b)) return `${where}.bricks: must be an object`;
    const extraBricks = unknownKey(b, ["enabled", "lives", "sound"], `${where}.bricks`);
    if (extraBricks) return extraBricks;
    for (const key of ["enabled", "sound"] as const) {
      if (b[key] === undefined) continue;
      if (typeof b[key] !== "boolean") return `${where}.bricks.${key}: must be true or false`;
      games.bricks[key] = b[key];
    }
    if (b.lives !== undefined) {
      const n = b.lives;
      if (typeof n !== "number" || !Number.isInteger(n) || n < 1 || n > 99) return `${where}.bricks.lives must be 1 to 99`;
      games.bricks.lives = n;
    }
  }
  if (input.dodger !== undefined) {
    const d = input.dodger;
    if (!isObject(d)) return `${where}.dodger: must be an object`;
    const extraDodger = unknownKey(d, ["enabled", "sound", "music"], `${where}.dodger`);
    if (extraDodger) return extraDodger;
    for (const key of ["enabled", "sound", "music"] as const) {
      if (d[key] === undefined) continue;
      if (typeof d[key] !== "boolean") return `${where}.dodger.${key}: must be true or false`;
      games.dodger[key] = d[key];
    }
  }
  if (input.blocks !== undefined) {
    const b = input.blocks;
    if (!isObject(b)) return `${where}.blocks: must be an object`;
    const extraBlocks = unknownKey(b, ["enabled", "start_level", "sound"], `${where}.blocks`);
    if (extraBlocks) return extraBlocks;
    for (const key of ["enabled", "sound"] as const) {
      if (b[key] === undefined) continue;
      if (typeof b[key] !== "boolean") return `${where}.blocks.${key}: must be true or false`;
      games.blocks[key] = b[key];
    }
    if (b.start_level !== undefined) {
      const n = b.start_level;
      if (typeof n !== "number" || !Number.isInteger(n) || n < 0 || n > 9) return `${where}.blocks.start_level must be 0 to 9`;
      games.blocks.start_level = n;
    }
  }
  return games;
}

/** One home screen: its own items and time limits (src/config.rs `HomeScreen`). */
export interface HomeScreen {
  id: string;
  /** Short title (menus on the TV, admin panel). */
  name: string;
  /** Exactly one home screen is the default (★). */
  default: boolean;
  home: HomeSettings;
  discs: DiscSettings;
  folders: FolderSettings[];
  websites: WebsiteSettings[];
  programs: ProgramSettings[];
  emulators: EmulatorSettings[];
  games: GameSettings;
  limits: LimitRule[];
}

export interface StatisticsSettings {
  enabled: boolean;
  keep_days: number;
}

/** Off: nothing on the TV turns the computer off, only the button on the computer. */
export interface PowerSettings {
  enabled: boolean;
}

export interface Config {
  version: number;
  home_screens: HomeScreen[];
  parental: ParentalSettings;
  screensaver: ScreensaverSettings;
  statistics: StatisticsSettings;
  power: PowerSettings;
}

export function newHomeScreen(id: string, name: string): HomeScreen {
  return {
    id,
    name,
    default: false,
    home: { title: "curator", show_title: true, language: "auto" },
    discs: { enabled: true, allow_disc_menu: true, audio_languages: [], subtitle_languages: [] },
    folders: [],
    websites: [],
    programs: [],
    emulators: [],
    games: { pong: { enabled: false, computer_level: "normal", points_to_win: 7 }, bricks: { enabled: false, lives: 5, sound: false }, blocks: { enabled: false, start_level: 0, sound: false }, dodger: { enabled: false, sound: false, music: false } },
    limits: [],
  };
}

export function defaultConfig(): Config {
  return {
    version: CONFIG_VERSION,
    home_screens: [{ ...newHomeScreen(V1_SCREEN_ID, "Home"), default: true }],
    parental: { pin_backoff_doubling: true },
    screensaver: { mode: "title", idle_minutes: 10, aerial_quality: "1080p_h264", aerial_videos: [] },
    statistics: { enabled: false, keep_days: 90 },
    power: { enabled: true },
  };
}

/** An id from a name that isn't in `taken` (lowercase letters, digits and -). */
export function uniqueId(name: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  const base =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 34) || "item";
  if (!used.has(base)) return base;
  for (let n = 2; ; n++) {
    const id = `${base}-${n}`;
    if (!used.has(id)) return id;
  }
}

/** A short title as typed: control characters and outer spaces removed; `null` if unusable. */
export function cleanScreenName(name: string): string | null {
  const clean = name.replace(/\p{Cc}/gu, "").trim();
  const chars = [...clean].length;
  return chars >= 1 && chars <= MAX_SCREEN_NAME_CHARS ? clean : null;
}

/** All folder, website and program ids (unique across home screens). */
export function allItemIds(config: Config): string[] {
  return config.home_screens.flatMap((s) => [...s.folders.map((f) => f.id), ...s.websites.map((w) => w.id), ...s.programs.map((p) => p.id), ...s.emulators.map((e) => e.id)]);
}

/**
 * A new home screen: a copy of the default one (nothing has to be entered twice), not the
 * default itself, with new folder and website ids. Returns the new config and the screen's id.
 */
export function addHomeScreen(config: Config, name: string): { config: Config; id: string } {
  const id = uniqueId(name, config.home_screens.map((s) => s.id));
  const source: HomeScreen = structuredClone(defaultScreen(config));
  const taken = new Set(allItemIds(config));
  const fresh = (old: string) => {
    const next = uniqueId(old, taken);
    taken.add(next);
    return next;
  };
  const screen: HomeScreen = {
    ...source,
    id,
    name,
    default: false,
    folders: source.folders.map((f) => ({ ...f, id: fresh(f.id) })),
    websites: source.websites.map((w) => ({ ...w, id: fresh(w.id) })),
    programs: source.programs.map((p) => ({ ...p, id: fresh(p.id) })),
    emulators: source.emulators.map((e) => ({ ...e, id: fresh(e.id) })),
  };
  return { config: { ...config, home_screens: [...config.home_screens, screen] }, id };
}

/** Make this one the default (★); the star moves, so there is always exactly one. */
export function makeDefaultScreen(config: Config, id: string): Config {
  return { ...config, home_screens: config.home_screens.map((s) => ({ ...s, default: s.id === id })) };
}

/** Remove a home screen; the default one can't be removed. */
export function removeHomeScreen(config: Config, id: string): Config {
  if (defaultScreen(config).id === id) return config;
  return { ...config, home_screens: config.home_screens.filter((s) => s.id !== id) };
}

/** The websites of all home screens (their logos). */
export function allWebsites(config: Config): WebsiteSettings[] {
  return config.home_screens.flatMap((s) => s.websites);
}

/** The default (★) home screen. */
export function defaultScreen(config: Config): HomeScreen {
  return config.home_screens.find((s) => s.default) ?? config.home_screens[0]!;
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

function hasControlChars(text: string): boolean {
  return /[\u0000-\u001f\u007f]/.test(text);
}

/** A path we hand to DOSBox or RetroArch, in double quotes in their configuration. */
function gamePathOk(path: unknown): path is string {
  return typeof path === "string" && isNormalizedAbsolutePath(path) && !hasControlChars(path) && !path.includes('"');
}

function validateDos(input: unknown, needed: boolean, out: DosOptions): string | null {
  if (!isObject(input)) return "dos: must be an object";
  const extra = unknownKey(input, ["drives", "autostart", "startup_script", "correct_aspect", "keep_saves"], "dos");
  if (extra) return extra;
  if (input.drives !== undefined) {
    if (!Array.isArray(input.drives)) return "dos.drives: must be a list";
    if (input.drives.length > MAX_DRIVES) return `dos.drives: at most ${MAX_DRIVES}`;
    for (const [i, drive] of input.drives.entries()) {
      if (!isObject(drive)) return `dos.drives[${i}]: must be an object`;
      const unknown = unknownKey(drive, ["letter", "path"], `dos.drives[${i}]`);
      if (unknown) return unknown;
      if (typeof drive.letter !== "string" || !DOS_DRIVE_LETTERS.includes(drive.letter)) return `dos.drives[${i}]: the drive letter must be A to Y`;
      if (out.drives.some((d) => d.letter === drive.letter)) return `dos.drives[${i}]: drive ${drive.letter} is used twice`;
      if (!gamePathOk(drive.path)) return `dos.drives[${i}]: must be an absolute path`;
      out.drives.push({ letter: drive.letter, path: drive.path });
    }
  }
  if (needed && out.drives.length === 0) return "dos.drives: add at least one drive";
  if (input.autostart !== undefined) {
    const a = input.autostart;
    if (typeof a !== "string" || [...a].length > MAX_AUTOSTART_CHARS || hasControlChars(a) || a.includes('"') || a.includes("..")) return "dos.autostart: not a file inside a drive";
    out.autostart = a;
  }
  if (input.startup_script !== undefined) {
    const script = input.startup_script;
    if (typeof script !== "string" || [...script].length > MAX_STARTUP_SCRIPT_CHARS) return `dos.startup_script: at most ${MAX_STARTUP_SCRIPT_CHARS} characters`;
    // A line starting with [ would begin a new section of DOSBox's configuration.
    if (script.split("\n").some((line) => hasControlChars(line) || line.trimStart().startsWith("["))) return "dos.startup_script: a line is not a DOS command";
    out.startup_script = script;
  }
  if (input.correct_aspect !== undefined) {
    if (typeof input.correct_aspect !== "boolean") return "dos.correct_aspect: must be true or false";
    out.correct_aspect = input.correct_aspect;
  }
  if (input.keep_saves !== undefined) {
    if (typeof input.keep_saves !== "boolean") return "dos.keep_saves: must be true or false";
    out.keep_saves = input.keep_saves;
  }
  return null;
}

function validateSnes(input: unknown, needed: boolean, out: SnesOptions): string | null {
  if (!isObject(input)) return "snes: must be an object";
  const extra = unknownKey(input, ["rom", "filter", "aspect", "integer_scale", "resume"], "snes");
  if (extra) return extra;
  if (input.rom !== undefined) {
    if (typeof input.rom !== "string") return "snes.rom: must be a path";
    out.rom = input.rom;
  }
  if (out.rom === "") {
    if (needed) return "snes.rom: choose a ROM file";
  } else {
    const extension = out.rom.includes(".") ? out.rom.split(".").pop()!.toLowerCase() : "";
    if (!gamePathOk(out.rom) || !SNES_ROM_EXTENSIONS.includes(extension)) return `snes.rom: an absolute path to a ${SNES_ROM_EXTENSIONS.join(", ")} file`;
  }
  if (input.filter !== undefined) {
    if (!SNES_FILTERS.includes(input.filter as SnesFilter)) return `snes.filter: one of ${SNES_FILTERS.join(", ")}`;
    out.filter = input.filter as SnesFilter;
  }
  if (input.aspect !== undefined) {
    if (!SNES_ASPECTS.includes(input.aspect as SnesAspect)) return `snes.aspect: one of ${SNES_ASPECTS.join(", ")}`;
    out.aspect = input.aspect as SnesAspect;
  }
  if (input.integer_scale !== undefined) {
    if (typeof input.integer_scale !== "boolean") return "snes.integer_scale: must be true or false";
    out.integer_scale = input.integer_scale;
  }
  if (input.resume !== undefined) {
    if (typeof input.resume !== "boolean") return "snes.resume: must be true or false";
    out.resume = input.resume;
  }
  return null;
}

function validateEmulator(input: unknown): EmulatorSettings | string {
  if (!isObject(input)) return "must be an object";
  const extra = unknownKey(input, ["id", "name", "system", "dos", "snes"], "emulator");
  if (extra) return extra;
  const idName = checkIdAndName(input.id, input.name);
  if (idName) return idName;
  if (input.system !== "dos" && input.system !== "snes") return "system: dos or snes";
  const emulator = newEmulator(input.id as string);
  emulator.name = input.name as string;
  emulator.system = input.system;
  const dos = validateDos(input.dos ?? {}, input.system === "dos", emulator.dos);
  if (dos) return dos;
  const snes = validateSnes(input.snes ?? {}, input.system === "snes", emulator.snes);
  if (snes) return snes;
  return emulator;
}

function validateProgram(input: unknown): ProgramSettings | string {
  if (!isObject(input)) return "must be an object";
  const extra = unknownKey(input, ["id", "name", "command", "args", "working_dir", "icon"], "program");
  if (extra) return extra;
  const idName = checkIdAndName(input.id, input.name);
  if (idName) return idName;
  const { command } = input;
  if (typeof command !== "string" || command === "" || [...command].length > MAX_COMMAND_CHARS) return `command: 1 to ${MAX_COMMAND_CHARS} characters`;
  if (hasControlChars(command) || command.startsWith("-")) return "command: not a program";
  if (command.startsWith("/")) {
    if (!isNormalizedAbsolutePath(command)) return "command: the path must not contain '.', '..' or empty parts";
  } else if (command.includes("/")) {
    return "command: an absolute path or a program name";
  } else if (command.includes(" ")) {
    return "command: put arguments into the list of arguments";
  }
  const program: ProgramSettings = { id: input.id as string, name: input.name as string, command, args: [], icon: "gamepad" };
  if (input.args !== undefined) {
    if (!Array.isArray(input.args)) return "args: must be a list";
    if (input.args.length > MAX_PROGRAM_ARGS) return `args: at most ${MAX_PROGRAM_ARGS}`;
    for (const arg of input.args) {
      if (typeof arg !== "string" || [...arg].length > MAX_ARG_CHARS || hasControlChars(arg)) return `args: at most ${MAX_ARG_CHARS} characters, no control characters`;
      program.args.push(arg);
    }
  }
  if (input.working_dir !== undefined) {
    const dir = input.working_dir;
    if (typeof dir !== "string" || !isNormalizedAbsolutePath(dir) || hasControlChars(dir)) return "working_dir: must be an absolute path";
    program.working_dir = dir;
  }
  if (input.icon !== undefined) {
    if (!PROGRAM_ICONS.includes(input.icon as ProgramIcon)) return `icon: one of ${PROGRAM_ICONS.join(", ")}`;
    program.icon = input.icon as ProgramIcon;
  }
  return program;
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

function validateLimits(value: unknown, at: string): LimitRule[] | string {
  if (!Array.isArray(value)) return `${at}: must be a list`;
  if (value.length > MAX_LIMITS) return `${at}: at most ${MAX_LIMITS} rules`;
  const rules: LimitRule[] = [];
  for (const [i, raw] of value.entries()) {
    const rule = validateLimit(raw);
    if (typeof rule === "string") return `${at}[${i}]: ${rule}`;
    rules.push(rule);
  }
  return rules;
}

/** `parental`; in version 1 it also had the time limits (`withLimits`). */
function validateParental(input: unknown, withLimits: boolean): { parental: ParentalSettings; limits: LimitRule[] } | string {
  if (!isObject(input)) return "parental: must be an object";
  const keys = ["pin_hash", "pin_length", "pin_backoff_doubling", ...(withLimits ? ["limits"] : [])];
  const extra = unknownKey(input, keys, "parental");
  if (extra) return extra;
  const parental: ParentalSettings = { pin_backoff_doubling: true };
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
  let limits: LimitRule[] = [];
  if (input.limits !== undefined) {
    const rules = validateLimits(input.limits, "parental.limits");
    if (typeof rules === "string") return rules;
    limits = rules;
  }
  return { parental, limits };
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
  const extra = unknownKey(input, ["id", "name", "path", "video", "audio", "images", "remember_recent", "extensions"], "folder");
  if (extra) return extra;
  const { id, name, path } = input;
  const idName = checkIdAndName(id, name);
  if (idName) return idName;
  if (typeof path !== "string" || !isNormalizedAbsolutePath(path)) {
    return `path "${String(path)}" must be an absolute path without '.', '..' or empty parts`;
  }
  const folder: FolderSettings = { id: id as string, name: name as string, path, video: false, audio: false, images: false, remember_recent: true };
  for (const key of ["video", "audio", "images"] as const) {
    if (input[key] === undefined) continue;
    if (typeof input[key] !== "boolean") return `${key}: must be true or false`;
    folder[key] = input[key];
  }
  if (input.remember_recent !== undefined) {
    if (typeof input.remember_recent !== "boolean") return "remember_recent: must be true or false";
    folder.remember_recent = input.remember_recent;
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

/** A home screen's sections (home, discs, folders, websites, games) into `screen`; `at` prefixes errors. */
function validateSections(input: Record<string, unknown>, at: string, screen: HomeScreen): string | null {
  if (input.home !== undefined) {
    const h = input.home;
    if (!isObject(h)) return `${at}home: must be an object`;
    const extra = unknownKey(h, ["title", "show_title", "language"], `${at}home`);
    if (extra) return extra;
    if (h.title !== undefined) {
      const problem = checkText(h.title);
      if (problem) return `${at}home.title: ${problem}`;
      screen.home.title = h.title as string;
    }
    if (h.show_title !== undefined) {
      if (typeof h.show_title !== "boolean") return `${at}home.show_title: must be true or false`;
      screen.home.show_title = h.show_title;
    }
    if (h.language !== undefined) {
      if (!(LANGUAGES as readonly unknown[]).includes(h.language)) return `${at}home.language must be one of ${LANGUAGES.join(", ")}`;
      screen.home.language = h.language as Language;
    }
  }
  if (input.discs !== undefined) {
    const d = input.discs;
    if (!isObject(d)) return `${at}discs: must be an object`;
    const extra = unknownKey(d, ["enabled", "allow_disc_menu", "audio_languages", "subtitle_languages"], `${at}discs`);
    if (extra) return extra;
    for (const key of ["enabled", "allow_disc_menu"] as const) {
      if (d[key] === undefined) continue;
      if (typeof d[key] !== "boolean") return `${at}discs.${key}: must be true or false`;
      screen.discs[key] = d[key];
    }
    for (const key of ["audio_languages", "subtitle_languages"] as const) {
      const result = languages(d[key], `${at}discs.${key}`);
      if (typeof result === "string") return result;
      screen.discs[key] = result;
    }
  }
  if (input.folders !== undefined) {
    if (!Array.isArray(input.folders)) return `${at}folders: must be a list`;
    if (input.folders.length > MAX_FOLDERS) return `${at}folders: at most ${MAX_FOLDERS} folders`;
    const ids = new Set<string>();
    for (const [i, raw] of input.folders.entries()) {
      const folder = validateFolder(raw);
      if (typeof folder === "string") return `${at}folders[${i}]: ${folder}`;
      if (ids.has(folder.id)) return `${at}folders[${i}]: id "${folder.id}" is used twice`;
      ids.add(folder.id);
      screen.folders.push(folder);
    }
  }
  if (input.websites !== undefined) {
    if (!Array.isArray(input.websites)) return `${at}websites: must be a list`;
    if (input.websites.length > MAX_WEBSITES) return `${at}websites: at most ${MAX_WEBSITES} websites`;
    const ids = new Set<string>();
    for (const [i, raw] of input.websites.entries()) {
      const site = validateWebsite(raw);
      if (typeof site === "string") return `${at}websites[${i}]: ${site}`;
      if (ids.has(site.id)) return `${at}websites[${i}]: id "${site.id}" is used twice`;
      ids.add(site.id);
      screen.websites.push(site);
    }
  }
  if (input.games !== undefined) {
    const games = validateGames(input.games, `${at}games`);
    if (typeof games === "string") return games;
    screen.games = games;
  }
  return null;
}

/** A version-1 config (one home screen) → version 2, like `ConfigV1::convert` in Rust. */
function validateV1(input: Record<string, unknown>): Validation {
  const top = unknownKey(input, ["version", "home", "parental", "discs", "folders", "websites", "screensaver", "games", "programs"], "config");
  if (top) return { ok: false, error: top };
  const config = defaultConfig();
  const screen = config.home_screens[0]!;
  const problem = validateSections(input, "", screen);
  if (problem) return { ok: false, error: problem };
  if (input.parental !== undefined) {
    const p = validateParental(input.parental, true);
    if (typeof p === "string") return { ok: false, error: p };
    config.parental = p.parental;
    screen.limits = p.limits;
  }
  // Old place of the games (before the Games tab): used if `games` is missing.
  if (input.programs !== undefined) {
    const old = validateGames(input.programs, "programs");
    if (typeof old === "string") return { ok: false, error: old };
    if (input.games === undefined) screen.games = old;
  }
  if (input.screensaver !== undefined) {
    const s = validateScreensaver(input.screensaver);
    if (typeof s === "string") return { ok: false, error: s };
    config.screensaver = s;
  }
  return { ok: true, config };
}

function validateStatistics(input: unknown): StatisticsSettings | string {
  if (!isObject(input)) return "statistics: must be an object";
  const extra = unknownKey(input, ["enabled", "keep_days"], "statistics");
  if (extra) return extra;
  const s: StatisticsSettings = { enabled: false, keep_days: 90 };
  if (input.enabled !== undefined) {
    if (typeof input.enabled !== "boolean") return "statistics.enabled: must be true or false";
    s.enabled = input.enabled;
  }
  if (input.keep_days !== undefined) {
    const n = input.keep_days;
    if (typeof n !== "number" || !Number.isInteger(n) || n < 1 || n > MAX_KEEP_DAYS) return `statistics.keep_days must be 1 to ${MAX_KEEP_DAYS}`;
    s.keep_days = n;
  }
  return s;
}

function validatePower(input: unknown): PowerSettings | string {
  if (!isObject(input)) return "power: must be an object";
  const extra = unknownKey(input, ["enabled"], "power");
  if (extra) return extra;
  if (input.enabled === undefined) return { enabled: true };
  if (typeof input.enabled !== "boolean") return "power.enabled: must be true or false";
  return { enabled: input.enabled };
}

/** Validate untrusted input and fill in defaults (version 2; version 1 is converted). */
export function validateConfig(input: unknown): Validation {
  if (!isObject(input)) return { ok: false, error: "config must be an object" };
  if (input.version === 1) return validateV1(input);
  const top = unknownKey(input, ["version", "home_screens", "parental", "screensaver", "statistics", "power"], "config");
  if (top) return { ok: false, error: top };
  if (input.version !== CONFIG_VERSION) {
    return { ok: false, error: `unsupported config version ${String(input.version)} (expected ${CONFIG_VERSION})` };
  }
  const config = defaultConfig();
  const list = input.home_screens;
  if (!Array.isArray(list) || list.length === 0 || list.length > MAX_HOME_SCREENS) {
    return { ok: false, error: `home_screens: 1 to ${MAX_HOME_SCREENS} home screens` };
  }
  config.home_screens = [];
  const folderIds = new Set<string>();
  const websiteIds = new Set<string>();
  const programIds = new Set<string>();
  const emulatorIds = new Set<string>();
  for (const [i, raw] of list.entries()) {
    const at = `home_screens[${i}]`;
    if (!isObject(raw)) return { ok: false, error: `${at}: must be an object` };
    const extra = unknownKey(raw, ["id", "name", "default", "home", "discs", "folders", "websites", "programs", "emulators", "games", "limits"], at);
    if (extra) return { ok: false, error: extra };
    const idName = checkIdAndName(raw.id, raw.name);
    if (idName) return { ok: false, error: `${at}: ${idName}` };
    if ([...(raw.name as string)].length > MAX_SCREEN_NAME_CHARS) {
      return { ok: false, error: `${at}.name: at most ${MAX_SCREEN_NAME_CHARS} characters` };
    }
    if (config.home_screens.some((s) => s.id === raw.id)) return { ok: false, error: `${at}: id "${String(raw.id)}" is used twice` };
    const screen = newHomeScreen(raw.id as string, raw.name as string);
    if (raw.default !== undefined) {
      if (typeof raw.default !== "boolean") return { ok: false, error: `${at}.default: must be true or false` };
      screen.default = raw.default;
    }
    const problem = validateSections(raw, `${at}.`, screen);
    if (problem) return { ok: false, error: problem };
    if (raw.limits !== undefined) {
      const rules = validateLimits(raw.limits, `${at}.limits`);
      if (typeof rules === "string") return { ok: false, error: rules };
      screen.limits = rules;
    }
    if (raw.programs !== undefined) {
      if (!Array.isArray(raw.programs)) return { ok: false, error: `${at}.programs: must be a list` };
      if (raw.programs.length > MAX_PROGRAMS) return { ok: false, error: `${at}.programs: at most ${MAX_PROGRAMS} programs` };
      for (const [n, item] of raw.programs.entries()) {
        const program = validateProgram(item);
        if (typeof program === "string") return { ok: false, error: `${at}.programs[${n}]: ${program}` };
        if (screen.programs.some((p) => p.id === program.id)) return { ok: false, error: `${at}.programs[${n}]: id "${program.id}" is used twice` };
        if (programIds.has(program.id)) return { ok: false, error: `${at}.programs: id "${program.id}" is used on another home screen` };
        screen.programs.push(program);
      }
      for (const p of screen.programs) programIds.add(p.id);
    }
    if (raw.emulators !== undefined) {
      if (!Array.isArray(raw.emulators)) return { ok: false, error: `${at}.emulators: must be a list` };
      if (raw.emulators.length > MAX_EMULATORS) return { ok: false, error: `${at}.emulators: at most ${MAX_EMULATORS}` };
      for (const [n, item] of raw.emulators.entries()) {
        const emulator = validateEmulator(item);
        if (typeof emulator === "string") return { ok: false, error: `${at}.emulators[${n}]: ${emulator}` };
        if (screen.emulators.some((e) => e.id === emulator.id)) return { ok: false, error: `${at}.emulators[${n}]: id "${emulator.id}" is used twice` };
        if (emulatorIds.has(emulator.id)) return { ok: false, error: `${at}.emulators: id "${emulator.id}" is used on another home screen` };
        screen.emulators.push(emulator);
      }
      for (const e of screen.emulators) emulatorIds.add(e.id);
    }
    // Folder and website ids are keys of our own files (recently played, logos).
    for (const f of screen.folders) {
      if (folderIds.has(f.id)) return { ok: false, error: `${at}.folders: id "${f.id}" is used on another home screen` };
      folderIds.add(f.id);
    }
    for (const w of screen.websites) {
      if (websiteIds.has(w.id)) return { ok: false, error: `${at}.websites: id "${w.id}" is used on another home screen` };
      websiteIds.add(w.id);
    }
    config.home_screens.push(screen);
  }
  if (config.home_screens.filter((s) => s.default).length !== 1) {
    return { ok: false, error: "home_screens: exactly one must be the default" };
  }
  if (input.parental !== undefined) {
    const p = validateParental(input.parental, false);
    if (typeof p === "string") return { ok: false, error: p };
    config.parental = p.parental;
  }
  if (input.screensaver !== undefined) {
    const s = validateScreensaver(input.screensaver);
    if (typeof s === "string") return { ok: false, error: s };
    config.screensaver = s;
  }
  if (input.statistics !== undefined) {
    const s = validateStatistics(input.statistics);
    if (typeof s === "string") return { ok: false, error: s };
    config.statistics = s;
  }
  if (input.power !== undefined) {
    const p = validatePower(input.power);
    if (typeof p === "string") return { ok: false, error: p };
    config.power = p;
  }
  return { ok: true, config };
}
