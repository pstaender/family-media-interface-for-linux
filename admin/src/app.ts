// The admin web server: serves the React app and a small JSON API.
//
// Security (see CLAUDE.md):
// - listens on all interfaces by default (see settings.ts); the Host header must be an IP
//   address, localhost or this computer's name (DNS rebinding),
// - the admin password can only be set up from this computer (not from the network),
// - session cookie: HttpOnly, SameSite=Strict,
// - changing requests need the header `X-Familynovice: 1` and a matching Origin (CSRF),
// - all input is validated before anything is written.

import { readdir, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import index from "./web/index.html";
import {
  Auth,
  clearedSessionCookie,
  sessionCookie,
  sessionToken,
} from "./auth";
import { isNormalizedAbsolutePath, validateConfig, defaultConfig, type Config } from "./config";
import { NOT_RUNNING, sendCommand } from "./control";
import { readJson, writeJsonAtomic } from "./files";
import { defaultLogoDir, readLogo, refreshLogos, updateLogo } from "./logos";
import { DEFAULT_HOST, isAllowedHost, isLoopbackAddress } from "./settings";

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
  /** Website logos ($XDG_CACHE_HOME/familynovice/logos). */
  logoDir?: string;
}

const MAX_BODY_BYTES = 64 * 1024;
const MAX_BROWSE_ENTRIES = 2000;

/**
 * List the subdirectories of `path` for the folder picker. Read-only (readdir/stat only);
 * hidden entries are skipped.
 */
export async function listDirectories(path: string): Promise<{ path: string; parent: string | null; dirs: string[] }> {
  const items = await readdir(path, { withFileTypes: true });
  const dirs: string[] = [];
  for (const item of items) {
    if (item.name.startsWith(".")) continue;
    let isDir = item.isDirectory();
    if (item.isSymbolicLink()) {
      isDir = await stat(join(path, item.name)).then((s) => s.isDirectory(), () => false);
    }
    if (isDir) dirs.push(item.name);
    if (dirs.length >= MAX_BROWSE_ENTRIES) break;
  }
  dirs.sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }));
  return { path, parent: path === "/" ? null : dirname(path), dirs };
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

export function createAdminServer(options: AdminOptions) {
  const configPath = join(options.configDir, "config.json");
  const socketPath = join(options.runtimeDir, "familynovice.sock");
  const auth = new Auth(join(options.configDir, "admin.json"));
  const logoDir = options.logoDir ?? defaultLogoDir();
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
      if (!isAllowedHost(host, server.port!, options.allowedHosts)) return json({ error: "Forbidden host." }, 403);
      if (mutating) {
        const origin = req.headers.get("origin");
        if (req.headers.get("x-familynovice") !== "1" || (origin !== null && origin !== `http://${host}`)) {
          return json({ error: "Forbidden." }, 403);
        }
      }
      if (login && !auth.isValid(sessionToken(req))) return json({ error: "Please log in." }, 401);
      return handler(req);
    };
  };

  const server = Bun.serve({
    hostname: options.hostname ?? DEFAULT_HOST,
    port: options.port,
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
            void refreshLogos(logoDir, v.config.websites);
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
            const reply = await sendCommand(socketPath, command);
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/unlock/end": {
        POST: api(
          async () => {
            const reply = await sendCommand(socketPath, "end_unlock");
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
          { mutating: true },
        ),
      },

      "/api/lock": {
        POST: api(
          async () => {
            const reply = await sendCommand(socketPath, "lock");
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
          },
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
            const site = (await loadConfig()).config.websites.find((w) => w.id === id);
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

      "/api/browse": {
        GET: api(async (req) => {
          const path = new URL(req.url).searchParams.get("path") || homedir();
          if (!isNormalizedAbsolutePath(path)) return json({ error: "Invalid path." }, 400);
          try {
            return json(await listDirectories(path));
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
        GET: api(async () => {
          const reply = await sendCommand(socketPath, "status", 2000);
          if (reply.ok) {
            return json({
              running: true,
              configError: reply.config_error ?? null,
              locked: reply.locked === true,
              lockReason: reply.lock_reason ?? null,
              remainingSecs: reply.remaining_secs ?? null,
              usedTodaySecs: reply.used_today_secs ?? 0,
              usedWeekSecs: reply.used_week_secs ?? 0,
              unlockedUntil: reply.unlocked_until ?? null,
            });
          }
          return json({ running: false, error: reply.error === NOT_RUNNING ? null : reply.error });
        }),
      },
    },
    fetch() {
      return json({ error: "Not found." }, 404);
    },
  });

  return server;
}
