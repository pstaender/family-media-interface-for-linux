// Admin password, sessions and login rate limiting.
//
// The password hash (argon2id, PHC string from Bun.password) is stored in admin.json next to
// config.json, readable only by the owner. The media center never reads this file.

import { createJsonExclusive, readJson, writeJsonAtomic } from "./files";

export const MIN_PASSWORD_LENGTH = 8;
const SESSION_TTL_MS = 8 * 60 * 60 * 1000;
const MAX_FAILURES = 5;
const FAILURE_WINDOW_MS = 15 * 60 * 1000;
const LOCKOUT_MS = 60 * 1000;

interface AdminFile {
  version: 1;
  password_hash: string;
}

export class Auth {
  private sessions = new Map<string, number>(); // token → expiry (ms)
  private failures: number[] = []; // timestamps of failed logins
  private lockedUntil = 0;

  constructor(
    private readonly adminFile: string,
    private readonly now: () => number = Date.now,
  ) {}

  async isSetUp(): Promise<boolean> {
    return (await this.load()) !== undefined;
  }

  private async load(): Promise<AdminFile | undefined> {
    const data = (await readJson(this.adminFile)) as AdminFile | undefined;
    if (data !== undefined && typeof data.password_hash !== "string") {
      throw new Error(`${this.adminFile} is damaged (no password hash)`);
    }
    return data;
  }

  /** First start: set the admin password. Fails if one is set already. */
  async setup(password: string): Promise<{ ok: true; token: string } | { ok: false; error: string }> {
    const problem = checkPassword(password);
    if (problem) return { ok: false, error: problem };
    const hash = await Bun.password.hash(password, { algorithm: "argon2id" });
    const created = await createJsonExclusive(this.adminFile, { version: 1, password_hash: hash });
    if (!created) return { ok: false, error: "An admin password is already set." };
    return { ok: true, token: this.newSession() };
  }

  async login(password: string): Promise<{ ok: true; token: string } | { ok: false; error: string }> {
    const now = this.now();
    if (now < this.lockedUntil) {
      const secs = Math.ceil((this.lockedUntil - now) / 1000);
      return { ok: false, error: `Too many wrong passwords. Try again in ${secs} seconds.` };
    }
    const admin = await this.load();
    if (!admin) return { ok: false, error: "No admin password is set yet." };
    if (await Bun.password.verify(password, admin.password_hash)) {
      this.failures = [];
      return { ok: true, token: this.newSession() };
    }
    this.failures = this.failures.filter((t) => now - t < FAILURE_WINDOW_MS);
    this.failures.push(now);
    if (this.failures.length >= MAX_FAILURES) {
      this.lockedUntil = now + LOCKOUT_MS;
      this.failures = [];
    }
    return { ok: false, error: "Wrong password." };
  }

  async changePassword(token: string | undefined, current: string, next: unknown) {
    if (!this.isValid(token)) return { ok: false as const, error: "Not logged in." };
    const admin = await this.load();
    if (!admin || !(await Bun.password.verify(current, admin.password_hash))) {
      return { ok: false as const, error: "The current password is wrong." };
    }
    const problem = checkPassword(next);
    if (problem || typeof next !== "string") return { ok: false as const, error: problem ?? "Invalid password." };
    const hash = await Bun.password.hash(next, { algorithm: "argon2id" });
    await writeJsonAtomic(this.adminFile, { version: 1, password_hash: hash });
    // Log out everywhere else.
    this.sessions.clear();
    return { ok: true as const, token: this.newSession() };
  }

  isValid(token: string | undefined): boolean {
    if (!token) return false;
    const expiry = this.sessions.get(token);
    if (expiry === undefined) return false;
    if (this.now() > expiry) {
      this.sessions.delete(token);
      return false;
    }
    return true;
  }

  logout(token: string | undefined) {
    if (token) this.sessions.delete(token);
  }

  private newSession(): string {
    const bytes = crypto.getRandomValues(new Uint8Array(32));
    const token = Buffer.from(bytes).toString("base64url");
    this.sessions.set(token, this.now() + SESSION_TTL_MS);
    return token;
  }
}

export function checkPassword(password: unknown): string | null {
  if (typeof password !== "string") return "Please enter a password.";
  if (password.length < MIN_PASSWORD_LENGTH) {
    return `The password needs at least ${MIN_PASSWORD_LENGTH} characters.`;
  }
  return null;
}

export const SESSION_COOKIE = "familynovice_session";

export function sessionCookie(token: string): string {
  return `${SESSION_COOKIE}=${token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=${SESSION_TTL_MS / 1000}`;
}

export function clearedSessionCookie(): string {
  return `${SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0`;
}

export function sessionToken(req: Request): string | undefined {
  const header = req.headers.get("cookie") ?? "";
  for (const part of header.split(";")) {
    const [name, ...rest] = part.trim().split("=");
    if (name === SESSION_COOKIE) return rest.join("=");
  }
  return undefined;
}
