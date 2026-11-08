import { chmod, mkdir, open, readFile, rename, stat } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";

/** Same rules as the Rust side: $CURATOR_CONFIG_DIR, else $XDG_CONFIG_HOME/curator. */
export function defaultConfigDir(): string {
  const override = process.env.CURATOR_CONFIG_DIR;
  if (override) return override;
  const base = process.env.XDG_CONFIG_HOME || join(homedir(), ".config");
  return join(base, "curator");
}

/**
 * curator was called "familynovice": take over its folders in the XDG config, data and
 * cache dirs once (only if the new one doesn't exist yet), and turn the old default title
 * into the new one. Same as `store::migrate_old_folders` in Rust; whichever part starts
 * first does it (renaming is atomic). Not for a config dir set with CURATOR_CONFIG_DIR.
 */
export async function migrateOldFolders(home = homedir(), env = process.env): Promise<void> {
  const bases = [
    env.CURATOR_CONFIG_DIR ? null : env.XDG_CONFIG_HOME || join(home, ".config"),
    env.XDG_DATA_HOME || join(home, ".local", "share"),
    env.XDG_CACHE_HOME || join(home, ".cache"),
  ];
  for (const [i, base] of bases.entries()) {
    if (!base) continue;
    const [oldDir, newDir] = [join(base, "familynovice"), join(base, "curator")];
    const oldIsDir = await stat(oldDir).then((s) => s.isDirectory(), () => false);
    const newExists = await stat(newDir).then(() => true, () => false);
    if (!oldIsDir || newExists) continue;
    try {
      await rename(oldDir, newDir);
      console.info(`renamed ${oldDir} to ${newDir}`);
    } catch (e) {
      console.warn(`could not rename ${oldDir}: ${(e as Error).message}`);
      continue;
    }
    if (i === 0) {
      const path = join(newDir, "config.json");
      const config = (await readJson(path).catch(() => undefined)) as { home?: { title?: unknown } } | undefined;
      if (config?.home?.title === "familynovice") {
        config.home.title = "curator";
        await writeJsonAtomic(path, config, 0o644);
      }
    }
  }
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
