import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { isMediaCenter, mediaCenterPids } from "../src/processes";

const status = (uid: number) => `Name:\tfamilynovice\nPPid:\t1\nUid:\t${uid}\t${uid}\t${uid}\t${uid}\n`;

test("only this user's media center counts", () => {
  expect(isMediaCenter("familynovice\n", status(1003), 1003)).toBe(true);
  expect(isMediaCenter("familynovice\n", status(1000), 1003)).toBe(false);
  expect(isMediaCenter("familynovice-we\n", status(1003), 1003)).toBe(false); // website window
  expect(isMediaCenter("familynovice-ki\n", status(1003), 1003)).toBe(false); // kiosk loop
  expect(isMediaCenter("bun\n", status(1003), 1003)).toBe(false);
  expect(isMediaCenter("familynovice\n", "Name:\tx\n", 1003)).toBe(false);
});

test("lists media centers from a /proc-like folder", async () => {
  const proc = mkdtempSync("/tmp/fn-proc-");
  const uid = process.getuid!();
  for (const [pid, comm, u] of [
    ["100", "familynovice", uid],
    ["200", "familynovice-we", uid],
    ["300", "familynovice", uid + 1],
    ["50", "familynovice", uid],
  ] as const) {
    mkdirSync(join(proc, pid));
    writeFileSync(join(proc, pid, "comm"), `${comm}\n`);
    writeFileSync(join(proc, pid, "status"), status(u));
  }
  mkdirSync(join(proc, "self"));
  expect(await mediaCenterPids(proc)).toEqual([50, 100]);
});
