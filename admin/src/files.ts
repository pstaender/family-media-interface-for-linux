import { chmod, mkdir, open, readFile, rename } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";

/** Same rules as the Rust side: $FAMILYNOVICE_CONFIG_DIR, else $XDG_CONFIG_HOME/familynovice. */
export function defaultConfigDir(): string {
  const override = process.env.FAMILYNOVICE_CONFIG_DIR;
  if (override) return override;
  const base = process.env.XDG_CONFIG_HOME || join(homedir(), ".config");
  return join(base, "familynovice");
}

/** Where the media center's control socket lives ($XDG_RUNTIME_DIR). */
export function defaultRuntimeDir(): string {
  return process.env.XDG_RUNTIME_DIR || tmpdir();
}

/** Read a JSON file; `undefined` if it doesn't exist. Throws on invalid JSON. */
export async function readJson(path: string): Promise<unknown> {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw e;
  }
}

/**
 * Write JSON atomically: temp file in the same directory, fsync, rename. The media center
 * never sees a half-written file.
 */
export async function writeJsonAtomic(path: string, value: unknown, mode = 0o600): Promise<void> {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const tmp = `${path}.${process.pid}.${Date.now()}.tmp`;
  const file = await open(tmp, "wx", mode);
  try {
    await file.writeFile(`${JSON.stringify(value, null, 2)}\n`);
    await file.sync();
  } finally {
    await file.close();
  }
  await chmod(tmp, mode);
  await rename(tmp, path);
}

/** Create a file only if it doesn't exist yet. Returns false if it already existed. */
export async function createJsonExclusive(path: string, value: unknown): Promise<boolean> {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  let file;
  try {
    file = await open(path, "wx", 0o600);
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "EEXIST") return false;
    throw e;
  }
  try {
    await file.writeFile(`${JSON.stringify(value, null, 2)}\n`);
    await file.sync();
  } finally {
    await file.close();
  }
  return true;
}
