// Injected OpenCode plugin that reports turn and interaction lifecycles to AcTrail.

import { createControlSocketReporter } from "../lib/actrail-control-socket-client.js";
import { createOpenCodeInteractionAdapter } from "../lib/actrail-lifecycle-adapter.js";
import { listPendingPermissions } from "../lib/actrail-opencode-permission-client.js";

const DEFAULT_PERMISSION_POLL_INTERVAL_MS = 250;

function permissionPollInterval() {
  const configured = Number(process.env.ACTRAIL_OPENCODE_PERMISSION_POLL_INTERVAL_MS);
  return Number.isFinite(configured) && configured >= 100
    ? configured
    : DEFAULT_PERMISSION_POLL_INTERVAL_MS;
}

export default async function ActrailOpenCodeLifecyclePlugin({ client, directory }) {
  if (process.env.ACTRAIL_AGENT_LIFECYCLE_ENABLED !== "true") return {};
  const traceId = process.env.ACTRAIL_TRACE_ID;
  const socketPath = process.env.ACTRAIL_CONTROL_SOCKET;
  if (!traceId || !socketPath) return {};

  const reporter = createControlSocketReporter({
    traceId,
    socketPath,
    observedAtUnixNanos: process.env.ACTRAIL_OBSERVED_AT_UNIX_NANOS,
  });

  const adapter = createOpenCodeInteractionAdapter({
    report: reporter,
  });

  // Poll permissions because some OpenCode versions omit V2 plugin events.
  let polling = false;
  const pollPermissions = async () => {
    if (polling) return;
    polling = true;
    try {
      await adapter.syncPermissions(await listPendingPermissions(client, directory));
    } catch {
      // Permission reconciliation is best-effort and must not disrupt OpenCode.
    } finally {
      polling = false;
    }
  };

  const permissionTimer = setInterval(() => void pollPermissions(), permissionPollInterval());
  permissionTimer.unref?.();

  return {
    "chat.headers": async (input) => {
      await adapter.modelStarted(input);
    },
    event: async ({ event }) => {
      const result = await adapter.handle(event);
      void pollPermissions();
      return result;
    },
  };
}
