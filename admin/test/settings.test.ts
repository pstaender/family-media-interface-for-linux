import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  DEFAULT_HOST,
  DEFAULT_PORT,
  isAllowedHost,
  isLoopbackAddress,
  loadServerSettings,
  parseArgs,
  parseSettingsFile,
} from "../src/settings";

const dir = mkdtempSync("/tmp/fn-admin-settings-");
afterAll(() => rmSync(dir, { recursive: true, force: true }));

test("defaults: port 8787 on all interfaces", async () => {
  const empty = mkdtempSync("/tmp/fn-admin-settings-empty-");
  const r = await loadServerSettings(empty, [], {});
  expect(r).toEqual({ ok: true, value: { port: DEFAULT_PORT, host: DEFAULT_HOST, allowedHosts: [] } });
  expect(DEFAULT_HOST).toBe("0.0.0.0");
  rmSync(empty, { recursive: true });
});

test("command line > environment > file > defaults", async () => {
  writeFileSync(join(dir, "admin-server.json"), JSON.stringify({ port: 9000, host: "127.0.0.1", allowed_hosts: ["TV.fritz.box"] }));
  const fileOnly = await loadServerSettings(dir, [], {});
  expect(fileOnly).toEqual({ ok: true, value: { port: 9000, host: "127.0.0.1", allowedHosts: ["tv.fritz.box"] } });
  const env = await loadServerSettings(dir, [], { CURATOR_ADMIN_PORT: "9100", CURATOR_ADMIN_HOST: "::" });
  expect(env.ok && env.value).toMatchObject({ port: 9100, host: "::" });
  const args = await loadServerSettings(dir, ["--port", "9200", "--host=0.0.0.0"], { CURATOR_ADMIN_PORT: "9100" });
  expect(args.ok && args.value).toMatchObject({ port: 9200, host: "0.0.0.0" });
});

test("rejects bad arguments, environment and files", async () => {
  for (const argv of [["--port"], ["--port", "0"], ["--port", "70000"], ["--port", "80a"], ["--host", "my.server"], ["--verbose"]]) {
    expect(parseArgs(argv).ok).toBe(false);
  }
  expect(parseArgs(["--help"])).toEqual({ ok: true, value: { help: true } });
  expect((await loadServerSettings(dir, [], { CURATOR_ADMIN_PORT: "-1" })).ok).toBe(false);
  for (const bad of [[], "x", { port: "8787x" }, { host: "example.com" }, { allowed_hosts: "tv" }, { allowed_hosts: ["a b"] }, { listen: 1 }]) {
    expect(parseSettingsFile(bad).ok).toBe(false);
  }
  writeFileSync(join(dir, "admin-server.json"), "{");
  expect((await loadServerSettings(dir, [], {})).ok).toBe(false);
});

test("Host header: IP addresses, localhost and this computer's names on our port only", () => {
  const ok = (host: string) => isAllowedHost(host, 8787, ["tv.fritz.box"], "Wohnzimmer");
  for (const host of ["127.0.0.1:8787", "localhost:8787", "192.168.1.20:8787", "[::1]:8787", "[fe80::1]:8787",
    "wohnzimmer:8787", "Wohnzimmer.local:8787", "tv.fritz.box:8787", "localhost.:8787"]) {
    expect(ok(host)).toBe(true);
  }
  for (const host of ["evil.example:8787", "192.168.1.20:80", "192.168.1.20", "127.0.0.1.evil.example:8787",
    "wohnzimmer.evil.example:8787", "[evil]:8787", "", ":8787", "localhost:8787:1", "user@127.0.0.1:8787"]) {
    expect(ok(host)).toBe(false);
  }
  expect(isAllowedHost("192.168.1.20", 80, [], "x")).toBe(true);
});

test("loopback client addresses", () => {
  for (const a of ["127.0.0.1", "127.1.2.3", "::1", "::ffff:127.0.0.1"]) expect(isLoopbackAddress(a)).toBe(true);
  for (const a of ["192.168.1.2", "::ffff:192.168.1.2", "fe80::1", "", undefined, "localhost"]) expect(isLoopbackAddress(a)).toBe(false);
});
