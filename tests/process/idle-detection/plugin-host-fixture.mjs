// A deterministic OpenCode host: it loads the real injected plugin and emits
// the subset of OpenCode server-plugin events needed by the process E2E.
const pluginPath = `${process.env.OPENCODE_CONFIG_DIR}/plugins/actrail-idle-plugin.js`;
const { default: plugin } = await import(pluginPath);

if (process.env.ACTRAIL_IDLE_DETECTION_ENABLED !== "true") {
  throw new Error("actrailctl did not enable the injected OpenCode plugin");
}

let pendingPermissions = [];
const host = await plugin({
  client: {
    _client: {
      get: async ({ url }) => ({
        data: url === "/api/permission/request" ? { data: pendingPermissions } : pendingPermissions,
      }),
    },
  },
  directory: process.cwd(),
});
if (typeof host.event !== "function") throw new Error("OpenCode plugin was not loaded");

const emit = async (event) => {
  if (!await host.event({ event })) {
    throw new Error(
      `plugin did not report ${event.type}; trace=${process.env.ACTRAIL_TRACE_ID} socket=${process.env.ACTRAIL_CONTROL_SOCKET}`,
    );
  }
};
const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const continueFromHarness = () => new Promise((resolve) => process.stdin.once("data", resolve));
const sessionID = "idle-e2e-session";

await emit({
  type: "message.updated",
  properties: { info: { id: "user-1", sessionID, role: "user" } },
});
await sleep(1_300);
console.log("PHASE idle-open");
await continueFromHarness();

pendingPermissions = [{ sessionID, id: "permission-1" }];
await emit({ type: "permission.asked", properties: { sessionID, id: "permission-1" } });
await sleep(1_300);
console.log("PHASE waiting-elapsed");
await continueFromHarness();

await emit({
  type: "permission.replied",
  properties: { sessionID, permissionID: "permission-1", reply: "once" },
});
pendingPermissions = [];
await sleep(1_300);
console.log("PHASE resumed-idle");
await continueFromHarness();

await emit({ type: "session.idle", properties: { sessionID } });
console.log("PHASE completed");
