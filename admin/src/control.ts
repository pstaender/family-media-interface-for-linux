// Client for the media center's control socket (src/control.rs).
// Protocol: one JSON object per line, e.g. {"command":"reload"} → {"ok":true}.

import { createConnection } from "node:net";

export interface Reply {
  ok: boolean;
  error?: string;
  [key: string]: unknown;
}

export const NOT_RUNNING = "The media center is not running.";

/** Send a command (`"reload"`, or an object like `{ command: "unlock", minutes: 30 }`). */
export function sendCommand(
  socketPath: string,
  command: string | Record<string, unknown>,
  timeoutMs = 5000,
): Promise<Reply> {
  const request = typeof command === "string" ? { command } : command;
  return new Promise((resolve) => {
    let buffer = "";
    let done = false;
    const finish = (reply: Reply) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      socket.destroy();
      resolve(reply);
    };
    const socket = createConnection(socketPath);
    const timer = setTimeout(
      () => finish({ ok: false, error: "The media center did not answer in time." }),
      timeoutMs,
    );
    socket.on("connect", () => socket.write(`${JSON.stringify(request)}\n`));
    socket.on("data", (chunk) => {
      buffer += chunk.toString();
      const newline = buffer.indexOf("\n");
      if (newline === -1) return;
      try {
        const reply = JSON.parse(buffer.slice(0, newline));
        finish(typeof reply?.ok === "boolean" ? reply : { ok: false, error: "Unexpected answer." });
      } catch {
        finish({ ok: false, error: "Unexpected answer from the media center." });
      }
    });
    socket.on("error", (e: NodeJS.ErrnoException) => {
      const notRunning = e.code === "ENOENT" || e.code === "ECONNREFUSED";
      finish({ ok: false, error: notRunning ? NOT_RUNNING : `Could not reach the media center: ${e.message}` });
    });
    socket.on("close", () => finish({ ok: false, error: "The media center closed the connection." }));
  });
}
