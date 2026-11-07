import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, statSync, readFileSync, writeFileSync } from "node:fs";
import { createServer, type Server as NetServer } from "node:net";
import { join } from "node:path";
import { createAdminServer } from "../src/app";

// Short paths: Unix socket paths are limited to 107 characters.
const configDir = mkdtempSync("/tmp/fn-admin-cfg-");
const runtimeDir = mkdtempSync("/tmp/fn-admin-run-");
const received: string[] = [];
let fakeMediaCenter: NetServer;
let server: ReturnType<typeof createAdminServer>;
let base: string;

beforeAll(async () => {
  // Fake media center control socket: records commands, answers ok.
  fakeMediaCenter = createServer((socket) => {
    socket.on("data", (data) => {
      for (const line of data.toString().split("\n").filter(Boolean)) {
        const { command } = JSON.parse(line);
        received.push(command);
        socket.write(`${JSON.stringify(command === "status" ? { ok: true, config_version: 1, config_error: null } : { ok: true })}\n`);
      }
    });
  });
  await new Promise<void>((resolve) => fakeMediaCenter.listen(join(runtimeDir, "familynovice.sock"), resolve));
  server = createAdminServer({ configDir, runtimeDir, port: 0 });
  base = `http://127.0.0.1:${server.port}`;
});

afterAll(() => {
  server.stop(true);
  fakeMediaCenter.close();
  rmSync(configDir, { recursive: true, force: true });
  rmSync(runtimeDir, { recursive: true, force: true });
});

let cookie = "";
const mutating = { "Content-Type": "application/json", "X-Familynovice": "1" };

async function call(method: string, path: string, body?: unknown, headers: Record<string, string> = mutating) {
  const res = await fetch(base + path, {
    method,
    headers: { ...headers, ...(cookie ? { Cookie: cookie } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const setCookie = res.headers.get("set-cookie");
  if (setCookie) cookie = setCookie.split(";")[0]!;
  return { status: res.status, data: (await res.json()) as Record<string, any> };
}

test("serves the admin page", async () => {
  const res = await fetch(`${base}/`);
  expect(res.status).toBe(200);
  expect(await res.text()).toContain('<div id="root">');
});

test("first start: not set up, nothing accessible", async () => {
  expect((await call("GET", "/api/state")).data).toEqual({ setUp: false, loggedIn: false });
  expect((await call("GET", "/api/config")).status).toBe(401);
});

test("setup needs the CSRF header and a long enough password", async () => {
  expect((await call("POST", "/api/setup", { password: "longenough" }, { "Content-Type": "application/json" })).status).toBe(403);
  expect((await call("POST", "/api/setup", { password: "short" })).status).toBe(400);
});

test("setup sets the password once, stored only as argon2 hash", async () => {
  const r = await call("POST", "/api/setup", { password: "correct horse" });
  expect(r.status).toBe(200);
  expect(cookie).toStartWith("familynovice_session=");
  const adminFile = join(configDir, "admin.json");
  const stored = readFileSync(adminFile, "utf8");
  expect(stored).toContain("$argon2id$");
  expect(stored).not.toContain("correct horse");
  expect(statSync(adminFile).mode & 0o777).toBe(0o600);

  const again = await call("POST", "/api/setup", { password: "another one" });
  expect(again.status).toBe(400);
});

test("folder picker needs a login", async () => {
  const saved = cookie;
  cookie = "";
  expect((await call("GET", "/api/browse?path=/")).status).toBe(401);
  cookie = saved;
});

test("rejects foreign Host headers (DNS rebinding)", async () => {
  const r = await call("GET", "/api/state", undefined, { Host: "evil.example:80" });
  expect(r.status).toBe(403);
});

test("rejects requests from another origin (CSRF)", async () => {
  const r = await call("PUT", "/api/config", { version: 1 }, { ...mutating, Origin: "http://evil.example" });
  expect(r.status).toBe(403);
});

test("reads default config, rejects invalid, saves and applies valid config", async () => {
  const get = await call("GET", "/api/config");
  expect(get.status).toBe(200);
  expect(get.data.config.discs.enabled).toBe(true);

  const bad = await call("PUT", "/api/config", { version: 1, discs: { audio_languages: ["german"] } });
  expect(bad.status).toBe(400);
  expect(bad.data.error).toContain("language code");

  received.length = 0;
  const good = await call("PUT", "/api/config", { version: 1, discs: { allow_disc_menu: false, audio_languages: ["deu"] } });
  expect(good.data).toEqual({ saved: true, applied: { ok: true } });
  expect(received).toEqual(["reload"]);
  const written = JSON.parse(readFileSync(join(configDir, "config.json"), "utf8"));
  expect(written).toEqual({
    version: 1,
    discs: { enabled: true, allow_disc_menu: false, audio_languages: ["deu"], subtitle_languages: [] },
    folders: [],
    websites: [],
  });
});

test("folder picker lists only visible directories, read-only", async () => {
  const base = mkdtempSync("/tmp/fn-admin-browse-");
  for (const d of ["Movies", "Music 10", "Music 2", ".hidden"]) mkdirSync(join(base, d));
  writeFileSync(join(base, "file.mkv"), "x");
  const r = await call("GET", `/api/browse?path=${encodeURIComponent(base)}`);
  expect(r.data.dirs).toEqual(["Movies", "Music 2", "Music 10"]);
  expect(r.data.parent).toBe("/tmp");
  expect((await call("GET", "/api/browse?path=relative")).status).toBe(400);
  expect((await call("GET", `/api/browse?path=${encodeURIComponent(base + "/../etc")}`)).status).toBe(400);
  const missing = await call("GET", `/api/browse?path=${encodeURIComponent(base + "/nope")}`);
  expect(missing.data.error).toBe("This folder doesn't exist.");
  rmSync(base, { recursive: true, force: true });
});

test("saves folders", async () => {
  const folders = [{ id: "movies", name: "Movies", path: "/srv/movies", video: true, audio: false, images: true }];
  const r = await call("PUT", "/api/config", { version: 1, folders });
  expect(r.data.saved).toBe(true);
  expect(JSON.parse(readFileSync(join(configDir, "config.json"), "utf8")).folders).toEqual(folders);
});

test("status comes from the media center", async () => {
  expect((await call("GET", "/api/status")).data).toEqual({ running: true, configError: null });
});

test("logout ends the session", async () => {
  await call("POST", "/api/logout");
  expect((await call("GET", "/api/config")).status).toBe(401);
});

test("wrong passwords lock the login for a while", async () => {
  for (let i = 0; i < 5; i++) {
    expect((await call("POST", "/api/login", { password: "wrong password" })).data.error).toBe("Wrong password.");
  }
  const locked = await call("POST", "/api/login", { password: "correct horse" });
  expect(locked.status).toBe(401);
  expect(locked.data.error).toContain("Too many wrong passwords");
});
