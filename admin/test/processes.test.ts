import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { isMediaCenter, mediaCenterPids } from "../src/processes";

const status = (uid: number) => `Name:\tcurator\nPPid:\t1\nUid:\t${uid}\t${uid}\t${uid}\t${uid}\n`;

test("only this user's media center counts", () => {
  expect(isMediaCenter("curator\n", status(1003), 1003)).toBe(true);
  expect(isMediaCenter("curator\n", status(1000), 1003)).toBe(false);
  expect(isMediaCenter("curator-web\n", status(1003), 1003)).toBe(false); // website window
  expect(isMediaCenter("curator-kiosk\n", status(1003), 1003)).toBe(false); // kiosk loop
  expect(isMediaCenter("bun\n", status(1003), 1003)).toBe(false);
  expect(isMediaCenter("curator\n", "Name:\tx\n", 1003)).toBe(false);
});

test("lists media centers from a /proc-like folder", async () => {
  const proc = mkdtempSync("/tmp/fn-proc-");
  const uid = process.getuid!();
  for (const [pid, comm, u] of [
    ["100", "curator", uid],
    ["200", "curator-web", uid],
    ["300", "curator", uid + 1],
    ["50", "curator", uid],
  ] as const) {
    mkdirSync(join(proc, pid));
    writeFileSync(join(proc, pid, "comm"), `${comm}\n`);
    writeFileSync(join(proc, pid, "status"), status(u));
  }
  mkdirSync(join(proc, "self"));
  expect(await mediaCenterPids(proc)).toEqual([50, 100]);
});
