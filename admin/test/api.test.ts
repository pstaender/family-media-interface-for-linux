import { afterAll, beforeAll, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, statSync, readFileSync, writeFileSync } from "node:fs";
import { createServer, type Server as NetServer } from "node:net";
import { join } from "node:path";
import { createAdminServer } from "../src/app";

// Short paths: Unix socket paths are limited to 107 characters.
const configDir = mkdtempSync("/tmp/fn-admin-cfg-");
const runtimeDir = mkdtempSync("/tmp/fn-admin-run-");
const received: string[] = [];
const requests: Record<string, unknown>[] = [];
let fakeMediaCenter: NetServer;
let server: ReturnType<typeof createAdminServer>;
let base: string;
let runningPids: number[] = [4242];

beforeAll(async () => {
  // Fake media center control socket: records commands, answers ok.
  fakeMediaCenter = createServer((socket) => {
    socket.on("data", (data) => {
      for (const line of data.toString().split("\n").filter(Boolean)) {
        const request = JSON.parse(line);
        const { command } = request;
        received.push(command);
        requests.push(request);
        const status = { ok: true, pid: 4242, config_version: 1, config_error: null, locked: false, lock_reason: null, remaining_secs: 600, used_today_secs: 1200, used_week_secs: 5400, unlocked_until: null };
        socket.write(`${JSON.stringify(command === "status" ? status : { ok: true })}\n`);
      }
    });
  });
  await new Promise<void>((resolve) => fakeMediaCenter.listen(join(runtimeDir, "curator.sock"), resolve));
  // Own logo/aerial folders: the tests must never touch the real cache.
  server = createAdminServer({
    configDir,
    runtimeDir,
    port: 0,
    hostname: "127.0.0.1",
    logoDir: join(configDir, "logos"),
    aerialDir: join(configDir, "aerial"),
    listMediaCenters: async () => runningPids,
  });
  base = `http://127.0.0.1:${server.port}`;
});

afterAll(() => {
  server.stop(true);
  fakeMediaCenter.close();
  rmSync(configDir, { recursive: true, force: true });
  rmSync(runtimeDir, { recursive: true, force: true });
});

let cookie = "";
const mutating = { "Content-Type": "application/json", "X-Curator": "1" };

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
  expect((await call("GET", "/api/state")).data).toEqual({ setUp: false, loggedIn: false, setupAllowed: true });
  expect((await call("GET", "/api/config")).status).toBe(401);
});

test("the first password can't be set from another device", async () => {
  const dir = mkdtempSync("/tmp/fn-admin-remote-");
  const remote = createAdminServer({ configDir: dir, runtimeDir, port: 0, hostname: "127.0.0.1", clientAddress: "192.168.1.30", aerialDir: join(dir, "aerial") });
  const url = `http://127.0.0.1:${remote.port}`;
  try {
    const state = await (await fetch(`${url}/api/state`)).json();
    expect(state).toEqual({ setUp: false, loggedIn: false, setupAllowed: false });
    const res = await fetch(`${url}/api/setup`, { method: "POST", headers: mutating, body: JSON.stringify({ password: "correct horse" }) });
    expect(res.status).toBe(403);
    expect(existsSync(join(dir, "admin.json"))).toBe(false);
  } finally {
    remote.stop(true);
    rmSync(dir, { recursive: true, force: true });
  }
});

test("setup needs the CSRF header and a long enough password", async () => {
  expect((await call("POST", "/api/setup", { password: "longenough" }, { "Content-Type": "application/json" })).status).toBe(403);
  expect((await call("POST", "/api/setup", { password: "short" })).status).toBe(400);
});

test("setup sets the password once, stored only as argon2 hash", async () => {
  const r = await call("POST", "/api/setup", { password: "correct horse" });
  expect(r.status).toBe(200);
  expect(cookie).toStartWith("curator_session=");
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

test("rejects foreign Host headers (DNS rebinding), accepts IP addresses", async () => {
  expect((await call("GET", "/api/state", undefined, { Host: "evil.example:80" })).status).toBe(403);
  expect((await call("GET", "/api/state", undefined, { Host: `evil.example:${server.port}` })).status).toBe(403);
  expect((await call("GET", "/api/state", undefined, { Host: "192.168.1.20:1" })).status).toBe(403);
  expect((await call("GET", "/api/state", undefined, { Host: `192.168.1.20:${server.port}` })).status).toBe(200);
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
    home: { title: "curator", show_title: true, language: "auto" },
    parental: { pin_backoff_doubling: true, limits: [] },
    discs: { enabled: true, allow_disc_menu: false, audio_languages: ["deu"], subtitle_languages: [] },
    folders: [],
    websites: [],
    screensaver: { mode: "title", idle_minutes: 10, aerial_quality: "1080p_h264", aerial_videos: [] },
    games: { pong: { enabled: false, computer_level: "normal", points_to_win: 7 }, bricks: { enabled: false, lives: 5, sound: false } },
  });
});

test("restart is passed to the media center", async () => {
  received.length = 0;
  const r = await call("POST", "/api/restart");
  expect(r.data).toEqual({ ok: true });
  expect(received).toEqual(["restart"]);
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
  expect((await call("GET", "/api/status")).data).toEqual({
    running: true,
    extraInstances: 0,
    extraPids: [],
    configError: null,
    locked: false,
    lockReason: null,
    remainingSecs: 600,
    usedTodaySecs: 1200,
    usedWeekSecs: 5400,
    unlockedUntil: null,
    remote: null,
  });
});

test("warns about media centers that don't get the changes", async () => {
  runningPids = [4242, 5555];
  const { data } = await call("GET", "/api/status");
  expect(data.extraInstances).toBe(1);
  expect(data.extraPids).toEqual([5555]);
  runningPids = [4242];
  expect((await call("GET", "/api/status")).data.extraInstances).toBe(0);
});

test("PIN: 4-8 digits, stored only as hash, never sent to the browser, kept on save", async () => {
  for (const bad of ["123", "123456789", "12a4", 1234]) {
    expect((await call("POST", "/api/pin", { pin: bad })).status).toBe(400);
  }
  expect((await call("POST", "/api/pin", { pin: "2468" })).data.saved).toBe(true);
  const file = readFileSync(join(configDir, "config.json"), "utf8");
  expect(file).toContain("$argon2id$");
  expect(file).not.toContain("2468");
  expect(JSON.parse(file).parental.pin_length).toBe(4);

  const got = await call("GET", "/api/config");
  expect(got.data.pinSet).toBe(true);
  expect(got.data.pinAutoSubmit).toBe(true);
  expect(JSON.stringify(got.data)).not.toContain("argon2");

  // A normal save keeps the PIN; the browser can't set the hash directly.
  const limits = [{ type: "budget", minutes: 60, scope: { kind: "every_day" } }];
  await call("PUT", "/api/config", { ...got.data.config, parental: { pin_backoff_doubling: false, limits } });
  const after = JSON.parse(readFileSync(join(configDir, "config.json"), "utf8"));
  expect(after.parental.pin_hash).toStartWith("$argon2id$");
  expect(after.parental.pin_length).toBe(4);
  expect(after.parental.limits).toEqual(limits);
  expect(after.parental.pin_backoff_doubling).toBe(false);
  const sneaky = await call("PUT", "/api/config", { version: 1, parental: { pin_hash: "$argon2id$x" } });
  expect(sneaky.status).toBe(400);

  await call("DELETE", "/api/pin");
  expect((await call("GET", "/api/config")).data.pinSet).toBe(false);
  const removed = JSON.parse(readFileSync(join(configDir, "config.json"), "utf8")).parental;
  expect(removed.pin_hash).toBeUndefined();
  expect(removed.pin_length).toBeUndefined();
});

test("unlock and lock are forwarded to the media center", async () => {
  requests.length = 0;
  expect((await call("POST", "/api/unlock", { minutes: 30 })).data).toEqual({ ok: true });
  expect((await call("POST", "/api/unlock", { rest_of_day: true })).data).toEqual({ ok: true });
  expect((await call("POST", "/api/unlock/end")).data).toEqual({ ok: true });
  expect((await call("POST", "/api/lock")).data).toEqual({ ok: true });
  expect(requests).toEqual([
    { command: "unlock", minutes: 30 },
    { command: "unlock", rest_of_day: true },
    { command: "end_unlock" },
    { command: "lock" },
  ]);
  requests.length = 0;
  expect((await call("POST", "/api/remote/calibrate")).data).toEqual({ ok: true });
  expect(requests).toEqual([{ command: "calibrate_remote" }]);
  for (const bad of [{}, { minutes: 0 }, { minutes: 2000 }, { minutes: "30" }]) {
    expect((await call("POST", "/api/unlock", bad)).status).toBe(400);
  }
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
