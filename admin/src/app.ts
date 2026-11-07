// The admin web server: serves the React app and a small JSON API.
//
// Security (see CLAUDE.md):
// - listens on 127.0.0.1 only; the Host header must be 127.0.0.1/localhost (DNS rebinding),
// - session cookie: HttpOnly, SameSite=Strict,
// - changing requests need the header `X-Familynovice: 1` and a matching Origin (CSRF),
// - all input is validated before anything is written.

import { join } from "node:path";
import index from "./web/index.html";
import {
  Auth,
  clearedSessionCookie,
  sessionCookie,
  sessionToken,
} from "./auth";
import { validateConfig, defaultConfig } from "./config";
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

function json(data: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return Response.json(data, {
    status,
    headers: { "Cache-Control": "no-store", ...headers },
  });
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
          let stored: unknown;
          try {
            stored = await readJson(configPath);
          } catch (e) {
            return json({ config: defaultConfig(), error: `config.json is not valid JSON: ${(e as Error).message}` });
          }
          if (stored === undefined) return json({ config: defaultConfig() });
          const v = validateConfig(stored);
          return v.ok
            ? json({ config: v.config })
            : json({ config: defaultConfig(), error: `config.json is invalid: ${v.error}` });
        }),
        PUT: api(
          async (req) => {
            const body = await readBody(req);
            const v = validateConfig(body);
            if (!v.ok) return json({ error: v.error }, 400);
            await writeJsonAtomic(configPath, v.config, 0o644);
            const applied = await sendCommand(socketPath, "reload");
            return json({ saved: true, applied });
          },
          { mutating: true },
        ),
      },

      "/api/status": {
        GET: api(async () => {
          const reply = await sendCommand(socketPath, "status", 2000);
          if (reply.ok) return json({ running: true, configError: reply.config_error ?? null });
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
