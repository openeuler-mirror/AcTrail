// Acceptance-only pause inside a real OpenCode task, before model dispatch.
import { access, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout } from "node:timers/promises";

export default async function HangGatePlugin() {
  const root = process.env.ACTRAIL_ACCEPTANCE_GATE_DIR;
  if (!root) throw new Error("acceptance gate directory is required");
  let token;
  let calls = 0;
  return {
    "chat.params": async (input) => {
      if (input.agent === "title") return;
      const config = JSON.parse(await readFile(join(root, "gate.json"), "utf8"));
      if (config.session !== input.sessionID) return;
      if (token !== config.token) { token = config.token; calls = 0; }
      calls += 1;
      if (calls <= config.skip) return;
      const marker = join(root, `${input.sessionID}.gate`);
      try { await access(marker); return; } catch {}
      await writeFile(marker, String(Date.now()));
      while (true) {
        try { await access(join(root, "release")); break; } catch {}
        await setTimeout(25);
      }
    },
    "chat.headers": async (input) => {
      if (input.agent !== "title") {
        await writeFile(join(root, `${input.sessionID}.model`), String(Date.now()));
      }
    },
  };
}
