// Entry point: `bun run dev` (development) or `bun run start`.
// Env: FAMILYNOVICE_ADMIN_PORT (default 8787), FAMILYNOVICE_CONFIG_DIR, XDG_CONFIG_HOME,
//      XDG_RUNTIME_DIR.

import { createAdminServer } from "./app";
import { defaultConfigDir, defaultRuntimeDir } from "./files";

const port = Number(process.env.FAMILYNOVICE_ADMIN_PORT ?? 8787);
const configDir = defaultConfigDir();

const server = createAdminServer({
  configDir,
  runtimeDir: defaultRuntimeDir(),
  port,
  development: process.env.NODE_ENV !== "production",
});

console.log(`familynovice admin: http://127.0.0.1:${server.port}/`);
console.log(`config directory:   ${configDir}`);
