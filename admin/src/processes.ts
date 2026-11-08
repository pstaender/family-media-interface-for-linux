// Finding running media centers (to warn when more than one runs).
//
// The admin server runs as the media center's user, so it can see that user's processes
// in /proc. Only processes named exactly "curator" (the media center; the website
// window "curator-web" and the kiosk loop "curator-kiosk" don't) of this user count.

import { readFile, readdir } from "node:fs/promises";

/** From /proc/<pid>/comm + status: is this our user's media center? */
export function isMediaCenter(comm: string, status: string, uid: number): boolean {
  const realUid = /^Uid:\s+(\d+)/m.exec(status)?.[1];
  return comm.trim() === "curator" && realUid !== undefined && Number(realUid) === uid;
}

/** Pids of this user's running media centers. */
export async function mediaCenterPids(proc = "/proc"): Promise<number[]> {
  const uid = process.getuid?.() ?? -1;
  const pids: number[] = [];
  for (const name of await readdir(proc).catch(() => [] as string[])) {
    if (!/^\d+$/.test(name)) continue;
    try {
      const [comm, status] = await Promise.all([
        readFile(`${proc}/${name}/comm`, "utf8"),
        readFile(`${proc}/${name}/status`, "utf8"),
      ]);
      if (isMediaCenter(comm, status, uid)) pids.push(Number(name));
    } catch {
      // gone meanwhile, or not ours
    }
  }
  return pids.sort((a, b) => a - b);
}
