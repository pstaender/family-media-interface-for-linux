// pm2 configuration for the curator admin panel (alternative to the systemd user service
// that scripts/install.sh sets up; use only one of them).
//
//   cd admin && bun install && cd ..
//   pm2 start ecosystem.config.js        # start
//   pm2 logs curator-admin          # logs (shows the URLs it is reachable at)
//   pm2 save && pm2 startup              # start again after a reboot
//
// Run pm2 as the media center's user: the panel writes that user's config and talks to the
// media center over its control socket in $XDG_RUNTIME_DIR.
//
// Port and listen address: `args` below (--port, --host), or <config dir>/admin-server.json:
//   {"port": 8787, "host": "0.0.0.0", "allowed_hosts": ["tv.fritz.box"]}
// Default: port 8787 on all interfaces (reachable from other devices in the network).

const { execFileSync } = require("node:child_process");
const { existsSync } = require("node:fs");
const { homedir } = require("node:os");
const { join } = require("node:path");

/** Bun: $BUN_PATH, ~/.bun/bin/bun or the one in PATH. */
function findBun() {
  if (process.env.BUN_PATH) return process.env.BUN_PATH;
  const local = join(homedir(), ".bun", "bin", "bun");
  if (existsSync(local)) return local;
  try {
    return execFileSync("sh", ["-c", "command -v bun"], { encoding: "utf8" }).trim();
  } catch {
    throw new Error("bun not found: install it (https://bun.sh) or set BUN_PATH");
  }
}

module.exports = {
  apps: [
    {
      name: "curator-admin",
      cwd: join(__dirname, "admin"),
      script: findBun(),
      interpreter: "none",
      // e.g. ["src/server.ts", "--port", "8080"] or [..., "--host", "127.0.0.1"] (only this computer)
      args: ["src/server.ts"],
      // Sessions live in memory: exactly one instance, never cluster mode.
      exec_mode: "fork",
      instances: 1,
      autorestart: true,
      restart_delay: 5000,
      max_restarts: 10,
      env: {
        NODE_ENV: "production",
        // No transpiler cache outside curator's folders.
        BUN_RUNTIME_TRANSPILER_CACHE_PATH: "0",
        // pm2 started at boot (pm2 startup) may lack it; the control socket lives there.
        XDG_RUNTIME_DIR: process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid()}`,
      },
    },
  ],
};
