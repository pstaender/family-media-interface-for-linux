// The admin web server: serves the React app and a small JSON API.
//
// Security (see CLAUDE.md):
// - listens on 127.0.0.1 only; the Host header must be 127.0.0.1/localhost (DNS rebinding),
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

export interface AdminOptions {
  configDir: string;
  /** Directory of the media center's control socket ($XDG_RUNTIME_DIR). */
  runtimeDir: string;
  port: number;
  hostname?: string;
  development?: boolean;
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
      const allowedHosts = [`127.0.0.1:${server.port}`, `localhost:${server.port}`];
      if (!allowedHosts.includes(host)) return json({ error: "Forbidden host." }, 403);
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
    hostname: options.hostname ?? "127.0.0.1",
    port: options.port,
    development: options.development ?? false,
    routes: {
      "/": index,

      "/api/state": {
        GET: api(
          async (req) =>
            json({ setUp: await auth.isSetUp(), loggedIn: auth.isValid(sessionToken(req)) }),
          { login: false },
        ),
      },

      "/api/setup": {
        POST: api(
          async (req) => {
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
          const { pin_hash, pin_length: _length, ...parental } = config.parental;
          const result: Record<string, unknown> = {
            config: { ...config, parental },
            pinSet: pin_hash !== undefined,
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
            return json(await saveAndApply(v.config));
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

      "/api/lock": {
        POST: api(
          async () => {
            const reply = await sendCommand(socketPath, "lock");
            return reply.ok ? json({ ok: true }) : json({ error: reply.error }, 502);
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
