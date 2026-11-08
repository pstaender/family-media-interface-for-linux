// Disk images and ROMs uploaded in the admin panel:
// $XDG_DATA_HOME/curator/emulators/library/{dos,snes}/<name>. Data, not cache: a cleared
// cache must not take the games away.

import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readdir, rename, rm, stat, statfs } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { DOS_UPLOAD_EXTENSIONS, SNES_ROM_EXTENSIONS, type Config } from "./config";

export type LibraryKind = "dos" | "snes";

export const MAX_DOS_UPLOAD_BYTES = 4 * 1024 ** 3;
const MAX_SNES_UPLOAD_BYTES = 64 * 1024 ** 2;
const KEEP_FREE_BYTES = 1024 ** 3;
/** An upload is only in the settings after saving them; give that time. */
const UNUSED_FOR_MS = 24 * 3600 * 1000;

export function defaultLibraryDir(env = process.env): string {
  const data = env.XDG_DATA_HOME || join(homedir(), ".local", "share");
  return join(data, "curator", "emulators", "library");
}

/** A harmless file name of this kind from what the browser sent, or null. */
export function safeFileName(name: string, kind: LibraryKind): string | null {
  const base = name.split(/[/\\]/).pop() ?? "";
  const clean = base.replace(/[^A-Za-z0-9._() -]/g, "_").replace(/^[.\s]+/, "").trim();
  const dot = clean.lastIndexOf(".");
  if (dot <= 0) return null;
  const extension = clean.slice(dot + 1).toLowerCase();
  if (!(kind === "dos" ? DOS_UPLOAD_EXTENSIONS : SNES_ROM_EXTENSIONS).includes(extension)) return null;
  return `${clean.slice(0, Math.min(dot, 100))}.${extension}`;
}

export class UploadError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

/** Store an upload (never over an existing file) and return its path. */
export async function storeUpload(dir: string, kind: LibraryKind, name: string, length: number, body: ReadableStream<Uint8Array>): Promise<string> {
  const limit = kind === "dos" ? MAX_DOS_UPLOAD_BYTES : MAX_SNES_UPLOAD_BYTES;
  if (!Number.isInteger(length) || length <= 0) throw new UploadError("The file is empty.", 400);
  if (length > limit) throw new UploadError(`The file is too big (at most ${limit / 1024 ** 2} MB).`, 413);
  const folder = join(dir, kind);
  await mkdir(folder, { recursive: true });
  const space = await statfs(folder);
  if (space.bavail * space.bsize - length < KEEP_FREE_BYTES) throw new UploadError("There is not enough free space on the computer.", 507);
  const part = join(folder, `.${randomUUID()}.part`);
  const writer = Bun.file(part).writer();
  try {
    let received = 0;
    for await (const chunk of body as unknown as AsyncIterable<Uint8Array>) {
      received += chunk.length;
      if (received > length) throw new UploadError("The upload is bigger than announced.", 400);
      writer.write(chunk);
      await writer.flush();
    }
    await writer.end();
    if (received !== length) throw new UploadError("The upload was cut off.", 400);
    const dot = name.lastIndexOf(".");
    let target = join(folder, name);
    for (let n = 2; existsSync(target); n++) target = join(folder, `${name.slice(0, dot)}-${n}${name.slice(dot)}`);
    await rename(part, target);
    return target;
  } catch (e) {
    await Promise.resolve(writer.end()).catch(() => {});
    await rm(part, { force: true });
    throw e;
  }
}

/** Remove uploads no game uses any more (only inside the library, and not fresh ones). */
export async function pruneLibrary(dir: string, config: Config, now = Date.now()): Promise<number> {
  const used = new Set(config.home_screens.flatMap((s) => s.emulators.flatMap((e) => [e.snes.rom, ...e.dos.drives.map((d) => d.path)])));
  let removed = 0;
  for (const kind of ["dos", "snes"]) {
    const folder = join(dir, kind);
    const names = await readdir(folder).catch(() => [] as string[]);
    for (const name of names) {
      const path = join(folder, name);
      const info = await stat(path).catch(() => null);
      if (!info?.isFile() || used.has(path) || now - info.mtimeMs < UNUSED_FOR_MS) continue;
      await rm(path, { force: true });
      removed++;
    }
  }
  return removed;
}
