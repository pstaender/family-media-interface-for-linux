// Where the admin server listens and which Host names it answers to.
//
// Sources, highest priority first:
//   1. command line:  --port 8787  --host 0.0.0.0
//   2. environment:   CURATOR_ADMIN_PORT, CURATOR_ADMIN_HOST
//   3. file:          <config dir>/admin-server.json
//                     {"port": 8787, "host": "0.0.0.0", "allowed_hosts": ["tv.fritz.box"]}
//   4. defaults:      port 8787 on all interfaces (reachable from other devices in the network)

import { isIP } from "node:net";
import { hostname, networkInterfaces } from "node:os";
import { join } from "node:path";
import { HOST_NAME } from "./config";
import { readJson } from "./files";

export const DEFAULT_PORT = 8787;
export const DEFAULT_HOST = "0.0.0.0";
export const SETTINGS_FILE = "admin-server.json";

export interface ServerSettings {
  port: number;
  /** Address to listen on: an IP address ("0.0.0.0" = all IPv4 interfaces, "::" = all) or "localhost". */
  host: string;
  /** Extra names the server may be reached by (Host header), e.g. a router's DNS name. */
  allowedHosts: string[];
}

type Result<T> = { ok: true; value: T } | { ok: false; error: string };

export function parsePort(value: unknown, source: string): Result<number> {
  const n = typeof value === "string" && /^\d+$/.test(value) ? Number(value) : value;
  if (typeof n !== "number" || !Number.isInteger(n) || n < 1 || n > 65535) {
    return { ok: false, error: `${source}: the port must be a number from 1 to 65535` };
  }
  return { ok: true, value: n };
}

export function parseHost(value: unknown, source: string): Result<string> {
  if (typeof value !== "string" || (value !== "localhost" && isIP(value) === 0)) {
    return { ok: false, error: `${source}: the host must be an IP address (e.g. 0.0.0.0, 127.0.0.1, ::) or localhost` };
  }
  return { ok: true, value };
}

/** Validate the content of admin-server.json (all fields optional, unknown fields rejected). */
export function parseSettingsFile(value: unknown): Result<Partial<ServerSettings>> {
  const where = SETTINGS_FILE;
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { ok: false, error: `${where}: expected a JSON object` };
  }
  const out: Partial<ServerSettings> = {};
  for (const [key, v] of Object.entries(value)) {
    if (key === "port") {
      const r = parsePort(v, where);
      if (!r.ok) return r;
      out.port = r.value;
    } else if (key === "host") {
      const r = parseHost(v, where);
      if (!r.ok) return r;
      out.host = r.value;
    } else if (key === "allowed_hosts") {
      if (!Array.isArray(v) || !v.every((h) => typeof h === "string" && HOST_NAME.test(h.toLowerCase()))) {
        return { ok: false, error: `${where}: allowed_hosts must be a list of host names (e.g. "tv.fritz.box")` };
      }
      out.allowedHosts = v.map((h: string) => h.toLowerCase());
    } else {
      return { ok: false, error: `${where}: unknown setting "${key}" (known: port, host, allowed_hosts)` };
    }
  }
  return { ok: true, value: out };
}

export const USAGE = `Usage: bun src/server.ts [--port N] [--host ADDRESS]

  --port N        port (default ${DEFAULT_PORT}; env CURATOR_ADMIN_PORT)
  --host ADDRESS  address to listen on (default ${DEFAULT_HOST} = all interfaces, reachable
                  from other devices; 127.0.0.1 = only this computer;
                  env CURATOR_ADMIN_HOST)

Both can also be set in <config dir>/${SETTINGS_FILE}:
  {"port": ${DEFAULT_PORT}, "host": "${DEFAULT_HOST}", "allowed_hosts": ["tv.fritz.box"]}
`;

/** Command-line arguments. `help` is set for --help / -h. */
export function parseArgs(argv: string[]): Result<Partial<ServerSettings> & { help?: boolean }> {
  const out: Partial<ServerSettings> & { help?: boolean } = {};
  for (let i = 0; i < argv.length; i++) {
    let arg = argv[i]!;
    let value: string | undefined;
    const eq = arg.indexOf("=");
    if (arg.startsWith("--") && eq > 0) [arg, value] = [arg.slice(0, eq), arg.slice(eq + 1)];
    if (arg === "--help" || arg === "-h") {
      out.help = true;
      continue;
    }
    if (arg !== "--port" && arg !== "--host") return { ok: false, error: `unknown argument: ${arg}` };
    value ??= argv[++i];
    if (value === undefined) return { ok: false, error: `${arg} needs a value` };
    const r = arg === "--port" ? parsePort(value, arg) : parseHost(value, arg);
    if (!r.ok) return r;
    if (arg === "--port") out.port = r.value as number;
    else out.host = r.value as string;
  }
  return { ok: true, value: out };
}

/** Combine all sources (see the top of this file). */
export async function loadServerSettings(
  configDir: string,
  argv: string[],
  env: Record<string, string | undefined>,
): Promise<Result<ServerSettings> & { help?: boolean }> {
  const args = parseArgs(argv);
  if (!args.ok) return args;
  if (args.value.help) return { ok: true, value: { port: DEFAULT_PORT, host: DEFAULT_HOST, allowedHosts: [] }, help: true };

  let file: Partial<ServerSettings> = {};
  let stored: unknown;
  try {
    stored = await readJson(join(configDir, SETTINGS_FILE));
  } catch (e) {
    return { ok: false, error: `${SETTINGS_FILE} is not valid JSON: ${(e as Error).message}` };
  }
  if (stored !== undefined) {
    const r = parseSettingsFile(stored);
    if (!r.ok) return r;
    file = r.value;
  }

  let envPort: number | undefined;
  let envHost: string | undefined;
  if (env.CURATOR_ADMIN_PORT) {
    const r = parsePort(env.CURATOR_ADMIN_PORT, "CURATOR_ADMIN_PORT");
    if (!r.ok) return r;
    envPort = r.value;
  }
  if (env.CURATOR_ADMIN_HOST) {
    const r = parseHost(env.CURATOR_ADMIN_HOST, "CURATOR_ADMIN_HOST");
    if (!r.ok) return r;
    envHost = r.value;
  }

  return {
    ok: true,
    value: {
      port: args.value.port ?? envPort ?? file.port ?? DEFAULT_PORT,
      host: args.value.host ?? envHost ?? file.host ?? DEFAULT_HOST,
      allowedHosts: file.allowedHosts ?? [],
    },
  };
}

/**
 * Is `host` (a request's Host header) one of our own names? Protects against DNS rebinding:
 * a foreign web page can only reach us under its own domain name, never under an IP address,
 * `localhost` or this computer's name. The port must match too.
 */
export function isAllowedHost(host: string, port: number, extraNames: string[] = [], machineName = hostname()): boolean {
  const m = /^(\[[0-9a-fA-F:.]+\]|[^:[\]]+)(?::(\d+))?$/.exec(host);
  if (!m) return false;
  const name = m[1]!.toLowerCase().replace(/\.$/, "");
  const hostPort = m[2] === undefined ? 80 : Number(m[2]);
  if (hostPort !== port) return false;
  if (name.startsWith("[")) return isIP(name.slice(1, -1)) === 6;
  if (isIP(name) === 4 || name === "localhost") return true;
  const machine = machineName.toLowerCase();
  return name === machine || name === `${machine}.local` || extraNames.includes(name);
}

/** Is `address` (a client's IP address) this computer itself? */
export function isLoopbackAddress(address: string | undefined): boolean {
  if (!address) return false;
  const v4 = address.startsWith("::ffff:") ? address.slice(7) : address;
  return address === "::1" || (isIP(v4) === 4 && v4.startsWith("127."));
}

/** URLs to print at startup: localhost plus this computer's network addresses (if listening on them). */
export function reachableUrls(host: string, port: number): string[] {
  const url = (address: string) => `http://${address.includes(":") ? `[${address}]` : address}:${port}/`;
  if (host !== "0.0.0.0" && host !== "::") return [url(host === "localhost" ? "127.0.0.1" : host)];
  const urls = [url("127.0.0.1")];
  for (const list of Object.values(networkInterfaces())) {
    for (const a of list ?? []) {
      if (!a.internal && a.family === "IPv4") urls.push(url(a.address));
    }
  }
  return urls;
}
