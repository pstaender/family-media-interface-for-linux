// Entry point: `bun run dev` (development) or `bun run start`.
// Port and listen address: command line, env or admin-server.json (see settings.ts, --help).
// Env: CURATOR_ADMIN_PORT, CURATOR_ADMIN_HOST, CURATOR_CONFIG_DIR,
//      XDG_CONFIG_HOME, XDG_RUNTIME_DIR.

import { createAdminServer } from "./app";
import { defaultConfigDir, defaultRuntimeDir, migrateOldFolders } from "./files";
import { defaultLogoDir, refreshLogos } from "./logos";
import { readJson } from "./files";
import { allWebsites, validateConfig } from "./config";
import { join } from "node:path";
import { USAGE, loadServerSettings, reachableUrls } from "./settings";

await migrateOldFolders();
const configDir = defaultConfigDir();
const settings = await loadServerSettings(configDir, process.argv.slice(2), process.env);
if (settings.help) {
  console.log(USAGE);
  process.exit(0);
}
if (!settings.ok) {
  console.error(`curator admin: ${settings.error}\n\n${USAGE}`);
  process.exit(2);
}

const { port, host, allowedHosts } = settings.value;
const server = createAdminServer({
  configDir,
  runtimeDir: defaultRuntimeDir(),
  port,
  hostname: host,
  allowedHosts,
  development: process.env.NODE_ENV !== "production",
});

console.log("curator admin:");
for (const url of reachableUrls(host, server.port!)) console.log(`  ${url}`);
console.log(`config directory: ${configDir}`);

// Logos still missing (e.g. after an update) and screensaver videos not downloaded yet, in
// the background.
readJson(join(configDir, "config.json"))
  .then((stored) => {
    const v = stored === undefined ? null : validateConfig(stored);
    if (!v?.ok) return;
    server.aerial.sync(v.config.screensaver);
    return refreshLogos(defaultLogoDir(), allWebsites(v.config));
  })
  .catch((e) => console.warn(`logos: ${(e as Error).message}`));
