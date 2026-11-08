// The admin web server: serves the React app and a small JSON API.
//
// Security (see CLAUDE.md):
// - listens on all interfaces by default (see settings.ts); the Host header must be an IP
//   address, localhost or this computer's name (DNS rebinding),
// - the admin password can only be set up from this computer (not from the network),
// - session cookie: HttpOnly, SameSite=Strict,
// - changing requests need the header `X-Curator: 1` and a matching Origin (CSRF),
// - all input is validated before anything is written.

import { existsSync } from "node:fs";
import { readdir, stat } from "node:fs/promises";
import { homedir, hostname } from "node:os";
import { dirname, join } from "node:path";
import index from "./web/index.html";
import {
  Auth,
  clearedSessionCookie,
  sessionCookie,
  sessionToken,
} from "./auth";
import { checkAllowedHosts, DOS_IMAGE_EXTENSIONS, SNES_ROM_EXTENSIONS, allWebsites, isNormalizedAbsolutePath, validateConfig, defaultConfig, type Config } from "./config";
import { defaultStatisticsDir, deleteStatistics, readStatistics } from "./statistics";
import { NOT_RUNNING, sendCommand } from "./control";
import { readJson, writeJsonAtomic } from "./files";
import { defaultLogoDir, readLogo, refreshLogos, updateLogo } from "./logos";
import { Aerial, sourceFor } from "./aerial";
import { mediaCenterPids } from "./processes";
import { AERIAL_QUALITIES, type AerialQuality } from "./config";
import { DEFAULT_HOST, isAllowedHost, isLoopbackAddress, SETTINGS_FILE } from "./settings";
import { defaultLibraryDir, MAX_DOS_UPLOAD_BYTES, pruneLibrary, safeFileName, storeUpload, UploadError } from "./emulator-library";

export interface AdminOptions {
  configDir: string;
  /** Directory of the media center's control socket ($XDG_RUNTIME_DIR). */
  runtimeDir: string;
  port: number;
  /** Address to listen on (default: all interfaces). */
  hostname?: string;
  /** Extra Host names besides IP addresses, localhost and this computer's name. */
  allowedHosts?: string[];
  development?: boolean;
  /** Tests only: pretend requests come from this client address. */
  clientAddress?: string;
  /** Website logos ($XDG_CACHE_HOME/curator/logos). */
  logoDir?: string;
  /** Screensaver videos ($XDG_CACHE_HOME/curator/aerial). */
  aerialDir?: string;
  /** Usage statistics ($XDG_DATA_HOME/curator/statistics). */
  statisticsDir?: string;
  /** Uploaded disk images and ROMs ($XDG_DATA_HOME/curator/emulators/library). */
  libraryDir?: string;
  /** The udev rule that takes the remote's power key away from logind. */
  remotePowerRule?: string;
  /** Tests only: the running media centers' pids (default: from /proc). */
  listMediaCenters?: () => Promise<number[]>;
}

const MAX_BODY_BYTES = 64 * 1024;
const MAX_BROWSE_ENTRIES = 2000;

/**
 * List the subdirectories of `path` for the folder picker. Read-only (readdir/stat only);
 * hidden entries are skipped.
 */
/** The folders of `path`; with `extensions`, also the files with one of them (read-only). */
export async function listDirectories(path: string, extensions: string[] = []): Promise<{ path: string; parent: string | null; dirs: string[]; files: string[] }> {
  const items = await readdir(path, { withFileTypes: true });
  const dirs: string[] = [];
  const files: string[] = [];
  for (const item of items) {
    if (item.name.startsWith(".")) continue;
    let isDir = item.isDirectory();
    if (item.isSymbolicLink()) {
      isDir = await stat(join(path, item.name)).then((s) => s.isDirectory(), () => false);
    }
    if (isDir) dirs.push(item.name);
    else if (extensions.includes(item.name.split(".").pop()!.toLowerCase())) files.push(item.name);
    if (dirs.length + files.length >= MAX_BROWSE_ENTRIES) break;
  }
  const order = (a: string, b: string) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });
  dirs.sort(order);
  files.sort(order);
  return { path, parent: path === "/" ? null : dirname(path), dirs, files };
}

function json(data: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return Response.json(data, {
    status,
    headers: { "Cache-Control": "no-store", ...headers },
  });
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

async function readBody(req: Request): Promise<Record<string, unknown> | null> {
  const length = Number(req.headers.get("content-length") ?? "0");
  if (length > MAX_BODY_BYTES) return null;
  const text = await req.text();
  if (text.length > MAX_BODY_BYTES) return null;
  try {
    const value = JSON.parse(text);
    return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

/** Is `url` the first-choice source of this quality (not a fallback)? */
function sourceMatches(sources: Record<string, string>, quality: AerialQuality, url: string): boolean {
  const key = { "1080p_h264": "H2641080p", "1080p_hevc": "H2651080p", "4k_hevc": "H2654k" }[quality];
  return sources[key] === url || (quality === "4k_hevc" && sources.H2654k240 === url);
}

export function createAdminServer(options: AdminOptions) {
  const configPath = join(options.configDir, "config.json");
  const socketPath = join(options.runtimeDir, "curator.sock");
  const auth = new Auth(join(options.configDir, "admin.json"));
  const logoDir = options.logoDir ?? defaultLogoDir();
  const aerial = new Aerial(options.aerialDir);
  const statisticsDir = options.statisticsDir ?? defaultStatisticsDir();
  const libraryDir = options.libraryDir ?? defaultLibraryDir();
  /** Can change while running (Password & PIN → Admin interface). */
  let allowedHosts = options.allowedHosts ?? [];
  const serverSettingsPath = join(options.configDir, SETTINGS_FILE);
  const remotePowerRule = options.remotePowerRule ?? "/etc/udev/rules.d/71-curator-remote-power.rules";
  /** A home screen id from a request (checked like the config's ids), or undefined. */
  const screenParam = (value: unknown): string | undefined | null =>
    value === undefined || value === null || value === ""
      ? undefined
      : typeof value === "string" && /^[a-z0-9][a-z0-9-]{0,39}$/.test(value)
        ? value
        : null;
  const listMediaCenters = options.listMediaCenters ?? (() => mediaCenterPids());
  /** Media centers of this user other than the one answering on the control socket. */
  const strayMediaCenters = async (connectedPid: unknown): Promise<number[]> => {
    const pids = await listMediaCenters();
    return typeof connectedPid === "number" ? pids.filter((p) => p !== connectedPid) : pids;
  };
  const fromThisComputer = (req: Request): boolean =>
    isLoopbackAddress(options.clientAddress ?? server.requestIP(req)?.address);

  /** The stored config (defaults if missing; defaults plus an error text if invalid). */
  const loadConfig = async (): Promise<{ config: Config; error?: string }> => {
    let stored: unknown;
    try {
      stored = await readJson(configPath);
    } catch (e) {
      return { config: defaultConfig(), error: `config.json is not valid JSON: ${(e as Error).message}` };
    }
    if (stored === undefined) return { config: defaultConfig() };
    const v = validateConfig(stored);
    return v.ok ? { config: v.config } : { config: defaultConfig(), error: `config.json is invalid: ${v.error}` };
  };

  const saveAndApply = async (config: Config) => {
    await writeJsonAtomic(configPath, config, 0o644);
    const applied = await sendCommand(socketPath, "reload");
    return { saved: true, applied };
  };

  type Handler = (req: Request) => Promise<Response> | Response;

  /** Wrap an API handler with the host, CSRF and (optionally) login checks. */
  const api = (handler: Handler, { login = true, mutating = false } = {}): Handler => {
    return (req) => {
      const host = req.headers.get("host") ?? "";
      if (!isAllowedHost(host, server.port!, allowedHosts)) return json({ error: "Forbidden host." }, 403);
      if (mutating) {
        const origin = req.headers.get("origin");
        if (req.headers.get("x-curator") !== "1" || (origin !== null && origin !== `http://${host}`)) {
          return json({ error: "Forbidden." }, 403);
        }
      }
      if (login && !auth.isValid(sessionToken(req))) return json({ error: "Please log in." }, 401);
      return handler(req);
    };
  };

  const listensOn = options.hostname ?? DEFAULT_HOST;
  const server = Bun.serve({
    hostname: listensOn,
    port: options.port,
    // Disk images; every other route refuses big bodies itself before reading them.
    maxRequestBodySize: MAX_DOS_UPLOAD_BYTES + 1024 ** 2,
    development: options.development ?? false,
    routes: {
      "/": index,

      "/api/state": {
        GET: api(
          async (req) =>
            json({
              setUp: await auth.isSetUp(),
              loggedIn: auth.isValid(sessionToken(req)),
              // The first password can only be chosen on this computer.
              setupAllowed: fromThisComputer(req),
            }),
          { login: false },
        ),
      },

      "/api/setup": {
        POST: api(
          async (req) => {
            if (!fromThisComputer(req)) {
              return json({ error: "Set the admin password on the media center computer itself first." }, 403);
            }
            const body = await readBody(req);
            const result = await auth.setup(body?.password as string);
            if (!result.ok) return json({ error: result.error }, 400);
            return json({ ok: true }, 200, { "Set-Cookie": sessionCookie(result.token) });
          },
          { login: false, mutating: true },
        ),
      },

      "/api/login": {
        POST: api(
          async (req) => {
            const body = await readBody(req);
            const password = typeof body?.password === "string" ? body.password : "";
            const result = await auth.login(password);
            if (!result.ok) return json({ error: result.error }, 401);
            return json({ ok: true }, 200, { "Set-Cookie": sessionCookie(result.token) });
          },
          { login: false, mutating: true },
        ),
      },

      "/api/logout": {
        POST: api(
          (req) => {
            auth.logout(sessionToken(req));
            return json({ ok: true }, 200, { "Set-Cookie": clearedSessionCookie() });
          },
          { login: false, mutating: true },
        ),
      },

      "/api/password": {
        POST: api(
          async (req) => {
            const body = await readBody(req);
            const result = await auth.changePassword(
              sessionToken(req),
              typeof body?.current === "string" ? body.current : "",
              body?.next,
            );
            if (!result.ok) return json({ error: result.error }, 400);
            return json({ ok: true }, 200, { "Set-Cookie": sessionCookie(result.token) });
          },
          { mutating: true },
        ),
      },

      "/api/config": {
        GET: api(async () => {
          const { config, error } = await loadConfig();
          // The PIN hash never leaves the server; the browser only learns whether one is set.
          const { pin_hash, pin_length, ...parental } = config.parental;
          const result: Record<string, unknown> = {
            config: { ...config, parental },
            pinSet: pin_hash !== undefined,
            // PINs set before the length was stored can't be checked automatically.
            pinAutoSubmit: pin_hash !== undefined && pin_length !== undefined,
          };
          if (error) result.error = error;
          return json(result);
        }),
        PUT: api(
          async (req) => {
            const body = await readBody(req);
            if (isRecord(body?.parental) && ("pin_hash" in body.parental || "pin_length" in body.parental)) {
              return json({ error: "The PIN can only be changed via /api/pin." }, 400);
            }
            const v = validateConfig(body);
            if (!v.ok) return json({ error: v.error }, 400);
            // Keep the stored PIN.
            const stored = (await loadConfig()).config.parental;
            if (stored.pin_hash !== undefined) v.config.parental.pin_hash = stored.pin_hash;
            if (stored.pin_length !== undefined) v.config.parental.pin_length = stored.pin_length;
            const result = await saveAndApply(v.config);
            // In the background: the media center picks up new logo files by itself.
            void refreshLogos(logoDir, allWebsites(v.config));
            // Downloads / deletes the screensaver videos in the background.
            aerial.sync(v.config.screensaver);
            void pruneLibrary(libraryDir, v.config);
            return json(result);
          },
          { mutating: true },
        ),
      },

      "/api/pin": {
        POST: api(
          async (req) => {
            const body = await readBody(req);
            const pin = body?.pin;
            if (typeof pin !== "string" || !/^\d{4,8}$/.test(pin)) {
              return json({ error: "The PIN must have 4 to 8 digits." }, 400);
            }
            const { config } = await loadConfig();
            config.parental.pin_hash = await Bun.password.hash(pin, { algorithm: "argon2id" });
            config.parental.pin_length = pin.length;
            return json(await saveAndApply(config));
          },
          { mutating: true },
        ),
        DELETE: api(
          async () => {
            const { config } = await loadConfig();
            delete config.parental.pin_hash;
            delete config.parental.pin_length;
            return json(await saveAndApply(config));
          },
          { mutating: true },
        ),
      },

      "/api/unlock": {
        POST: api(
          async (req) => {
            const body = await readBody(req);
            const minutes = body?.minutes;
            const command =
              body?.rest_of_day === true
                ? { command: "unlock", rest_of_day: true }
                : typeof minutes === "number" && Number.isInteger(minutes) && minutes >= 1 && minutes <= 1440
                  ? { command: "unlock", minutes }
                  : null;
            if (!command) return json({ error: "Choose minutes (1-1440) or the rest of today." }, 400);
            const screen = screenParam(body?.home_screen);
            if (screen === null) return json({ error: "Invalid home screen." }, 400);
            const reply = await sendCommand(socketPath, screen ? { ...command, home_screen: screen } : command);
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/unlock/end": {
        POST: api(
          async (req) => {
            const screen = screenParam((await readBody(req))?.home_screen);
            if (screen === null) return json({ error: "Invalid home screen." }, 400);
            const reply = await sendCommand(socketPath, { command: "end_unlock", ...(screen ? { home_screen: screen } : {}) });
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/lock": {
        POST: api(
          async (req) => {
            const screen = screenParam((await readBody(req))?.home_screen);
            if (screen === null) return json({ error: "Invalid home screen." }, 400);
            const reply = await sendCommand(socketPath, { command: "lock", ...(screen ? { home_screen: screen } : {}) });
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/home-screen": {
        // Show this home screen on the TV now.
        POST: api(
          async (req) => {
            const screen = screenParam((await readBody(req))?.home_screen);
            if (!screen) return json({ error: "Choose a home screen." }, 400);
            const reply = await sendCommand(socketPath, { command: "switch_home_screen", home_screen: screen });
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/statistics": {
        // The last `days` days (1-366) of usage statistics.
        GET: api(async (req) => {
          const days = Number(new URL(req.url).searchParams.get("days") ?? "7");
          if (!Number.isInteger(days) || days < 1 || days > 366) return json({ error: "days must be 1 to 366." }, 400);
          return json({ entries: await readStatistics(statisticsDir, days) });
        }),
        // Delete all collected statistics (only our own day files).
        DELETE: api(
          async () => json({ ok: true, deleted: await deleteStatistics(statisticsDir) }),
          { mutating: true },
        ),
      },

      "/api/websites/:id/logo": {
        // The stored logo (preview in the website settings).
        GET: api(async (req) => {
          const id = (req as Request & { params: { id: string } }).params.id;
          if (!/^[a-z0-9][a-z0-9-]{0,39}$/.test(id)) return json({ error: "Invalid id." }, 400);
          const logo = await readLogo(logoDir, id);
          if (!logo) return json({ error: "No logo yet." }, 404);
          return new Response(new Blob([new Uint8Array(logo.bytes)]), {
            headers: {
              "Content-Type": logo.type,
              "Cache-Control": "no-store",
              "X-Content-Type-Options": "nosniff",
              ...(logo.meta ? { "X-Logo-Source": encodeURI(logo.meta.source) } : {}),
            },
          });
        }),
        // Look for the logo again now (of the saved website, never a URL from the request).
        POST: api(
          async (req) => {
            const id = (req as Request & { params: { id: string } }).params.id;
            const site = allWebsites((await loadConfig()).config).find((w) => w.id === id);
            if (!site) return json({ error: "Save the website first." }, 404);
            if (!site.show_logo) return json({ error: "The logo is turned off for this website." }, 400);
            try {
              const meta = await updateLogo(logoDir, site);
              return json({ ok: true, source: meta.source });
            } catch (e) {
              return json({ error: `No logo found: ${(e as Error).message}` }, 502);
            }
          },
          { mutating: true },
        ),
      },

      "/api/aerial": {
        // The aerial videos to choose from, with sizes for a quality, the chosen ones'
        // download state and the free space.
        GET: api(async (req) => {
          const q = new URL(req.url).searchParams.get("quality");
          const quality: AerialQuality = (AERIAL_QUALITIES as readonly unknown[]).includes(q) ? (q as AerialQuality) : "1080p_h264";
          const refresh = new URL(req.url).searchParams.get("refresh") === "1";
          const { videos, fetchedAt, error } = await aerial.videos(refresh);
          const urls = new Map(videos.map((v) => [v.id, sourceFor(v, quality)]));
          const sizes = await aerial.sizesFor([...urls.values()].filter((u): u is string => u !== null));
          return json({
            videos: videos.map((v) => {
              const url = urls.get(v.id);
              return {
                id: v.id,
                name: v.name,
                location: v.location,
                type: v.type,
                timeOfDay: v.timeOfDay,
                size: url ? (sizes.get(url) ?? null) : null,
                // This quality isn't available: the 4K file is used.
                fallback: url != null && !sourceMatches(v.sources, quality, url),
              };
            }),
            fetchedAt,
            error,
            downloads: await aerial.downloads(),
            freeBytes: await aerial.freeBytes(),
          });
        }),
      },

      "/api/restart": {
        // Stops what plays and starts the media center again (also picks up an update).
        POST: api(
          async () => {
            const reply = await sendCommand(socketPath, "restart");
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/remote/calibrate": {
        POST: api(
          async () => {
            const reply = await sendCommand(socketPath, "calibrate_remote");
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/gamepad/calibrate": {
        POST: api(
          async () => {
            const reply = await sendCommand(socketPath, "calibrate_gamepad");
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/emulators/upload": {
        POST: api(
          async (req) => {
            const params = new URL(req.url).searchParams;
            const kind = params.get("kind");
            if (kind !== "dos" && kind !== "snes") return json({ error: "Invalid file kind." }, 400);
            const name = safeFileName(params.get("name") ?? "", kind);
            if (name === null) return json({ error: "This kind of file can't be used here." }, 400);
            if (!req.body) return json({ error: "The file is empty." }, 400);
            try {
              const path = await storeUpload(libraryDir, kind, name, Number(req.headers.get("content-length") ?? "0"), req.body);
              return json({ path });
            } catch (e) {
              if (e instanceof UploadError) return json({ error: e.message }, e.status);
              return json({ error: `The file could not be stored: ${(e as Error).message}` }, 500);
            }
          },
          { mutating: true },
        ),
      },

      "/api/admin-interface": {
        GET: api(() =>
          json({
            allowedHosts,
            machineName: hostname(),
            // Listening only on this computer: other names lead nowhere.
            localOnly: listensOn === "localhost" || listensOn === "::1" || listensOn.startsWith("127."),
          }),
        ),
        PUT: api(
          async (req) => {
            const body = await readBody(req);
            const checked = checkAllowedHosts(body?.allowedHosts);
            if (!checked.ok) return json({ error: checked.error }, 400);
            const here = req.headers.get("host") ?? "";
            if (!isAllowedHost(here, server.port!, checked.hosts)) {
              return json({ error: `You are connected as ${here}, so this name can't be removed from here. Use another address, e.g. this computer's IP address.` }, 400);
            }
            // The other settings of the file stay as they are.
            const stored = await readJson(serverSettingsPath).catch(() => undefined);
            await writeJsonAtomic(serverSettingsPath, { ...(isRecord(stored) ? stored : {}), allowed_hosts: checked.hosts }, 0o644);
            allowedHosts = checked.hosts;
            return json({ saved: true, allowedHosts });
          },
          { mutating: true },
        ),
      },

      "/api/browse": {
        GET: api(async (req) => {
          const params = new URL(req.url).searchParams;
          const path = params.get("path") || homedir();
          if (!isNormalizedAbsolutePath(path)) return json({ error: "Invalid path." }, 400);
          // ?files=dos|snes: also the game files of that system (nothing else is ever listed).
          const kind = params.get("files");
          if (kind !== null && kind !== "dos" && kind !== "snes") return json({ error: "Invalid file kind." }, 400);
          const extensions = kind === "dos" ? DOS_IMAGE_EXTENSIONS : kind === "snes" ? SNES_ROM_EXTENSIONS : [];
          try {
            return json(await listDirectories(path, extensions));
          } catch (e) {
            const code = (e as NodeJS.ErrnoException).code;
            const reason =
              code === "ENOENT" ? "This folder doesn't exist." :
              code === "EACCES" ? "No permission to read this folder." :
              code === "ENOTDIR" ? "This is not a folder." : "This folder can't be read.";
            return json({ error: reason, path, parent: path === "/" ? null : dirname(path) }, 400);
          }
        }),
      },

      "/api/status": {
        GET: api(async (req) => {
          // ?home_screen=…: that home screen's usage and limits (default: the one on the TV).
          const screen = screenParam(new URL(req.url).searchParams.get("home_screen"));
          if (screen === null) return json({ error: "Invalid home screen." }, 400);
          const reply = await sendCommand(socketPath, { command: "status", ...(screen ? { home_screen: screen } : {}) }, 2000);
          // More than one media center: the one on the TV may not get the changes.
          const extraPids = await strayMediaCenters(reply.ok ? reply.pid : undefined);
          const extraInstances = extraPids.length;
          const remotePowerFree = existsSync(remotePowerRule);
          if (reply.ok) {
            return json({
              remotePowerFree,
              extraInstances,
              extraPids,
              running: true,
              configError: reply.config_error ?? null,
              homeScreen: reply.home_screen ?? null,
              activeHomeScreen: reply.active_home_screen ?? null,
              activeLocked: reply.active_locked === true,
              locked: reply.locked === true,
              lockReason: reply.lock_reason ?? null,
              remainingSecs: reply.remaining_secs ?? null,
              usedTodaySecs: reply.used_today_secs ?? 0,
              usedWeekSecs: reply.used_week_secs ?? 0,
              unlockedUntil: reply.unlocked_until ?? null,
              remote: reply.remote ?? null,
            });
          }
          return json({ running: false, remotePowerFree, extraInstances, extraPids, error: reply.error === NOT_RUNNING ? null : reply.error });
        }),
      },

      "/api/instances/stop-extra": {
        // End the media centers that aren't connected to the admin panel (same user only).
        POST: api(
          async () => {
            const reply = await sendCommand(socketPath, "status", 2000);
            if (!reply.ok || typeof reply.pid !== "number") {
              return json({ error: "None of them is connected to the admin panel. Log the media center account out and in again, or restart the computer." }, 409);
            }
            const stray = await strayMediaCenters(reply.pid);
            for (const pid of stray) {
              try {
                process.kill(pid, "SIGTERM");
              } catch {
                // already gone
              }
            }
            console.info(`ended stray media centers: ${stray.join(", ") || "none"}`);
            return json({ ok: true, ended: stray.length });
          },
          { mutating: true },
        ),
      },
    },
    fetch() {
      return json({ error: "Not found." }, 404);
    },
  });

  return Object.assign(server, { aerial });
}
