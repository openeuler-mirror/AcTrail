// Tests OpenCode event adaptation and control-socket reporting.
import assert from "node:assert/strict";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createOpenCodeInteractionAdapter } from "../../../deploy/agent-host/opencode/lib/actrail-idle-adapter.js";
import {
  createControlSocketReporter,
  decodeControlFields,
  encodeControlFields,
} from "../../../deploy/agent-host/opencode/lib/actrail-control-socket-client.js";

const events = [];
const adapter = createOpenCodeInteractionAdapter({
  taskId: "launch:42",
  report: (event) => events.push(event),
});

const userMessage = (sessionID, id) => ({
  type: "message.updated",
  properties: { info: { id, sessionID, role: "user" } },
});
const assistantPart = (sessionID, messageID) => ({
  type: "message.part.updated",
  properties: { sessionID, messageID },
});
const idle = (sessionID) => ({ type: "session.idle", properties: { sessionID } });

const opencodeDir = join(dirname(fileURLToPath(import.meta.url)), "../../../deploy/agent-host/opencode");
const pluginEntries = (await readdir(join(opencodeDir, "plugins")))
  .filter((entry) => /\.(?:js|ts)$/.test(entry))
  .sort();
assert.deepEqual(pluginEntries, ["actrail-idle-plugin.js"]);
const pluginModule = await import("../../../deploy/agent-host/opencode/plugins/actrail-idle-plugin.js");
assert.deepEqual(Object.keys(pluginModule), ["default"]);
assert.equal(typeof pluginModule.default, "function");

const priorIdleDetectionEnabled = process.env.ACTRAIL_IDLE_DETECTION_ENABLED;
process.env.ACTRAIL_IDLE_DETECTION_ENABLED = "false";
assert.deepEqual(await pluginModule.default({}), {});
if (priorIdleDetectionEnabled === undefined) delete process.env.ACTRAIL_IDLE_DETECTION_ENABLED;
else process.env.ACTRAIL_IDLE_DETECTION_ENABLED = priorIdleDetectionEnabled;

// Ordinary model/tool/session activity is not a user wait.
await adapter.handle({ type: "session.status", properties: { sessionID: "ses_a", status: "active" } });
await adapter.handle({ type: "message.part.updated", properties: { sessionID: "ses_a" } });
assert.equal(events.length, 0);

// Permission events can race ahead of the corresponding user message. The
// user message must not create a second lifecycle task while the permission
// already owns the active turn.
const lateContextEvents = [];
const lateContextAdapter = createOpenCodeInteractionAdapter({
  taskId: "launch:42",
  report: (event) => lateContextEvents.push(event),
});
await lateContextAdapter.handle({
  type: "permission.asked",
  properties: { sessionID: "ses_context", id: "per_context" },
});
await lateContextAdapter.handle({
  type: "message.updated",
  properties: {
    info: { id: "msg_context", sessionID: "ses_context", role: "user" },
  },
});
assert.deepEqual(
  lateContextEvents.filter((event) => event.type === "turn"),
  [
    { type: "turn", taskId: "launch:42:session:ses_context:turn:1", state: "started" },
  ],
);

// A user message starts a turn. Permission approval is an interaction inside
// that turn; idle while it is pending must not complete the turn.
await adapter.handle(userMessage("ses_a", "msg_a1"));
await adapter.handle(userMessage("ses_a", "msg_a1"));
assert.deepEqual(events, [
  { type: "turn", taskId: "launch:42:session:ses_a:turn:1", state: "started" },
]);
await adapter.handle({ type: "permission.asked", properties: { sessionID: "ses_a", id: "per_a" } });
await adapter.handle({ type: "permission.asked", properties: { sessionID: "ses_a", id: "per_a" } });
await adapter.handle(idle("ses_a"));
assert.equal(adapter.pendingCount(), 1);
await adapter.handle({ type: "permission.replied", properties: { sessionID: "ses_a", permissionID: "per_a", reply: "once" } });
assert.equal(events.at(-1).state, "resolved");
await adapter.handle(assistantPart("ses_a", "msg_a2"));
await adapter.handle(idle("ses_a"));
assert.deepEqual(events, [
  { type: "turn", taskId: "launch:42:session:ses_a:turn:1", state: "started" },
  { type: "interaction", taskId: "launch:42:session:ses_a:turn:1", interactionId: "opencode:ses_a:per_a", reason: "approval", state: "requested" },
  { type: "interaction", taskId: "launch:42:session:ses_a:turn:1", interactionId: "opencode:ses_a:per_a", reason: "approval", state: "resolved" },
  { type: "turn", taskId: "launch:42:session:ses_a:turn:1", state: "completed" },
]);

// Current OpenCode releases can emit the V2 permission namespace. The plugin
// hook may expose the payload as either properties (legacy hook envelope) or
// data (V2 event envelope); both forms must pause and resume the same turn.
await adapter.handle(userMessage("ses_v2", "msg_v2"));
await adapter.handle({
  type: "permission.v2.asked",
  data: {
    id: "per_v2",
    sessionID: "ses_v2",
    action: "bash",
    resources: ["pwd"],
  },
});
assert.equal(adapter.pendingCount(), 1);
assert.equal(events.at(-1).interactionId, "opencode:ses_v2:per_v2");
assert.equal(events.at(-1).state, "requested");
await adapter.handle({
  type: "permission.v2.replied",
  data: { sessionID: "ses_v2", requestID: "per_v2", reply: "once" },
});
assert.equal(adapter.pendingCount(), 0);
assert.equal(events.at(-1).interactionId, "opencode:ses_v2:per_v2");
assert.equal(events.at(-1).state, "resolved");
await adapter.handle(idle("ses_v2"));
assert.equal(events.at(-1).state, "completed");

// The V2 permission service is authoritative when an OpenCode server-plugin
// event bridge omits permission.v2 events. Reconciliation requests the
// interaction from its pending list and resolves it once it disappears.
await adapter.handle(userMessage("ses_polled", "msg_polled"));
await adapter.syncPermissions([{
  id: "per_polled",
  sessionID: "ses_polled",
  permission: "bash",
  patterns: ["pwd"],
}]);
assert.equal(adapter.pendingCount(), 1);
assert.equal(events.at(-1).interactionId, "opencode:ses_polled:per_polled");
assert.equal(events.at(-1).state, "requested");
await adapter.syncPermissions([]);
assert.equal(adapter.pendingCount(), 0);
assert.equal(events.at(-1).interactionId, "opencode:ses_polled:per_polled");
assert.equal(events.at(-1).state, "resolved");
await adapter.handle(idle("ses_polled"));
assert.equal(events.at(-1).state, "completed");

// A late duplicate after completion is still ignored, while a genuinely new
// message starts a new turn in the same CLI/session.
await adapter.handle(userMessage("ses_a", "msg_a1"));
assert.equal(events.at(-1).state, "completed");

// Questions use the same V2 namespace migration as permissions.
await adapter.handle(userMessage("ses_question_v2", "msg_question_v2"));
await adapter.handle({
  type: "question.v2.asked",
  properties: { id: "que_v2", sessionID: "ses_question_v2", questions: [] },
});
assert.equal(adapter.pendingCount(), 1);
assert.equal(events.at(-1).reason, "input");
await adapter.handle({
  type: "question.v2.rejected",
  properties: { requestID: "que_v2", sessionID: "ses_question_v2" },
});
assert.equal(adapter.pendingCount(), 0);
assert.equal(events.at(-1).state, "resolved");
await adapter.handle(idle("ses_question_v2"));
assert.equal(events.at(-1).state, "completed");
await adapter.handle(userMessage("ses_a", "msg_a3"));
await adapter.handle(assistantPart("ses_a", "msg_a4"));
await adapter.handle({ type: "session.status", properties: { sessionID: "ses_a", status: { type: "idle" } } });
assert.equal(events.at(-2).state, "started");
assert.equal(events.at(-1).state, "completed");

// Question input and rejection are distinct from approval. Rejection can
// complete at the next idle because no post-allow continuation is required.
await adapter.handle(userMessage("ses_b", "msg_b1"));
await adapter.handle({ type: "question.asked", properties: { sessionID: "ses_b", requestID: "que_b" } });
await adapter.handle({ type: "question.rejected", properties: { sessionID: "ses_b", requestID: "que_b" } });
await adapter.handle(idle("ses_b"));
assert.equal(adapter.pendingCount(), 0);
assert.equal(events.at(-3).reason, "input");
assert.equal(events.at(-3).state, "requested");
assert.equal(events.at(-2).state, "resolved");
assert.equal(events.at(-1).state, "completed");

// Session idle/status/error are not interaction replies. They must leave the
// permission pending until its matching reply arrives; after a reject, the
// next authoritative idle completes the turn.
await adapter.handle({ type: "permission.asked", properties: { sessionID: "ses_c", id: "per_c" } });
await adapter.handle(idle("ses_c"));
await adapter.handle({ type: "session.status", properties: { sessionID: "ses_c", status: { type: "idle" } } });
await adapter.handle({ type: "session.error", properties: { sessionID: "ses_c" } });
assert.equal(adapter.pendingCount(), 1);
assert.equal(events.at(-1).state, "requested");
await adapter.handle({ type: "permission.replied", properties: { sessionID: "ses_c", permissionID: "per_c", reply: "reject" } });
await adapter.handle(idle("ses_c"));
assert.equal(adapter.pendingCount(), 0);
assert.equal(events.at(-2).state, "resolved");
assert.equal(events.at(-1).state, "completed");

// Explicit deletion resolves the interaction and closes that session turn.
await adapter.handle({ type: "permission.asked", properties: { sessionID: "ses_c", id: "per_c2" } });
await adapter.handle({ type: "session.deleted", properties: { sessionID: "ses_c" } });
assert.equal(adapter.pendingCount(), 0);
assert.equal(events.at(-2).interactionId, "opencode:ses_c:per_c2");
assert.equal(events.at(-2).state, "resolved");
assert.equal(events.at(-1).state, "completed");

// A failed report must not commit local state. The same event can retry after
// the socket recovers, including a reply whose first resolution report failed.
const recoveryEvents = [];
let failNextReport = true;
const recoveryAdapter = createOpenCodeInteractionAdapter({
  taskId: "launch:recovery",
  report: async (event) => {
    if (failNextReport) {
      failNextReport = false;
      throw new Error("temporary control socket failure");
    }
    recoveryEvents.push(event);
  },
});
assert.equal(
  await recoveryAdapter.handle(userMessage("ses_recovery", "msg_recovery")),
  false,
);
assert.equal(recoveryEvents.length, 0);
assert.equal(
  await recoveryAdapter.handle(userMessage("ses_recovery", "msg_recovery")),
  true,
);
assert.equal(recoveryEvents[0].state, "started");
assert.equal(
  await recoveryAdapter.handle({
    type: "permission.asked",
    properties: { sessionID: "ses_recovery", id: "per_recovery" },
  }),
  true,
);
assert.equal(recoveryAdapter.pendingCount(), 1);
failNextReport = true;
assert.equal(
  await recoveryAdapter.handle({
    type: "permission.replied",
    properties: { sessionID: "ses_recovery", permissionID: "per_recovery" },
  }),
  false,
);
assert.equal(recoveryAdapter.pendingCount(), 1);
assert.equal(
  await recoveryAdapter.handle({
    type: "permission.replied",
    properties: { sessionID: "ses_recovery", permissionID: "per_recovery" },
  }),
  true,
);
assert.equal(recoveryAdapter.pendingCount(), 0);
assert.equal(await recoveryAdapter.handle(idle("ses_recovery")), true);
assert.equal(recoveryEvents.at(-1).state, "completed");

// The adapter also defends against custom reporters explicitly returning
// false; the production entry point logs and rethrows reporter failures.
let falseReportCalls = 0;
const falseResultAdapter = createOpenCodeInteractionAdapter({
  taskId: "launch:false-result",
  report: async () => {
    falseReportCalls += 1;
    return false;
  },
});
assert.equal(
  await falseResultAdapter.handle(userMessage("ses_false", "msg_false")),
  false,
);
assert.equal(falseReportCalls, 1);
assert.equal(
  await falseResultAdapter.handle(userMessage("ses_false", "msg_false")),
  false,
);
assert.equal(falseReportCalls, 2);

// Permission-list reconciliation must recover even when the first turn-start
// report failed. The pending permission is itself enough to create and retry
// the turn; reconciliation must not require an already-active local turn.
const pollRecoveryEvents = [];
let failFirstPollReport = true;
const pollRecoveryAdapter = createOpenCodeInteractionAdapter({
  taskId: "launch:poll-recovery",
  report: async (event) => {
    if (failFirstPollReport) {
      failFirstPollReport = false;
      throw new Error("temporary control socket failure");
    }
    pollRecoveryEvents.push(event);
  },
});
const pendingPermission = [{
  id: "per_poll_recovery",
  sessionID: "ses_poll_recovery",
  permission: "bash",
  patterns: ["pwd"],
}];
assert.equal(await pollRecoveryAdapter.syncPermissions(pendingPermission), false);
assert.equal(pollRecoveryAdapter.pendingCount(), 0);
assert.equal(await pollRecoveryAdapter.syncPermissions(pendingPermission), true);
assert.deepEqual(pollRecoveryEvents.map((event) => event.state), ["started", "requested"]);
assert.equal(pollRecoveryAdapter.pendingCount(), 1);

// Session deletion must report failure when any pending interaction cleanup
// fails. Successful cleanups are removed and are not sent again on retry.
const deleteEvents = [];
let failDeleteResolution = true;
const deleteAdapter = createOpenCodeInteractionAdapter({
  taskId: "launch:delete-recovery",
  report: async (event) => {
    if (event.state === "resolved" && event.interactionId.endsWith("delete-fail") && failDeleteResolution) {
      failDeleteResolution = false;
      throw new Error("temporary cleanup failure");
    }
    deleteEvents.push(event);
  },
});
await deleteAdapter.handle(userMessage("ses_delete", "msg_delete"));
await deleteAdapter.handle({
  type: "permission.asked",
  properties: { sessionID: "ses_delete", id: "delete-fail" },
});
await deleteAdapter.handle({
  type: "permission.asked",
  properties: { sessionID: "ses_delete", id: "delete-ok" },
});
assert.equal(deleteAdapter.pendingCount(), 2);
assert.equal(
  await deleteAdapter.handle({ type: "session.deleted", properties: { sessionID: "ses_delete" } }),
  false,
);
assert.equal(deleteAdapter.pendingCount(), 1);
assert.equal(
  deleteEvents.filter((event) => event.state === "resolved" && event.interactionId?.endsWith("delete-ok")).length,
  1,
);
assert.equal(deleteEvents.some((event) => event.state === "completed"), false);
assert.equal(
  await deleteAdapter.handle({ type: "session.deleted", properties: { sessionID: "ses_delete" } }),
  true,
);
assert.equal(deleteAdapter.pendingCount(), 0);
assert.equal(
  deleteEvents.filter((event) => event.state === "resolved" && event.interactionId?.endsWith("delete-ok")).length,
  1,
);
assert.equal(
  deleteEvents.filter((event) => event.state === "resolved" && event.interactionId?.endsWith("delete-fail")).length,
  1,
);
assert.equal(deleteEvents.at(-1).state, "completed");

// The production reporter uses the daemon's native length-prefixed UDS
// protocol directly. UTF-8 lengths are byte lengths, not JS string lengths.
assert.deepEqual(
  decodeControlFields(encodeControlFields(["ascii", "中文", ""])),
  ["ascii", "中文", ""],
);
const controlTemp = await mkdtemp(join(tmpdir(), "actrail-opencode-control-"));
const controlSocket = join(controlTemp, "control.sock");
const controlRequests = [];
const controlServer = createServer({ allowHalfOpen: true }, (socket) => {
  const chunks = [];
  socket.once("data", (chunk) => {
    chunks.push(chunk);
    const fields = decodeControlFields(Buffer.concat(chunks));
    controlRequests.push(fields);
    if (fields[4] === "bad-interaction") {
      socket.end(encodeControlFields(["error", "fixture_error", "rejected by fixture"]));
      return;
    }
    const reply = fields[0] === "report_turn_lifecycle"
      ? "reply_turn_lifecycle_recorded"
      : "reply_user_interaction_recorded";
    socket.end(encodeControlFields([reply]));
  });
});
await new Promise((resolve, reject) => {
  controlServer.once("error", reject);
  controlServer.listen(controlSocket, resolve);
});
let fixtureNanos = 1_800_000_000_000_000_000n;
const reportToControlSocket = createControlSocketReporter({
  traceId: "trace-42",
  socketPath: controlSocket,
  now: () => fixtureNanos++,
});
await reportToControlSocket({ type: "turn", taskId: "任务-1", state: "started" });
await reportToControlSocket({
  type: "interaction",
  taskId: "任务-1",
  interactionId: "approval-1",
  reason: "approval",
  state: "requested",
});
await assert.rejects(
  reportToControlSocket({
    type: "interaction",
    taskId: "任务-1",
    interactionId: "bad-interaction",
    reason: "approval",
    state: "requested",
  }),
  /fixture_error: rejected by fixture/,
);
assert.deepEqual(controlRequests[0].slice(0, 6), [
  "report_turn_lifecycle",
  controlRequests[0][1],
  "42",
  "任务-1",
  "started",
  "1800000000000000000",
]);
assert.match(controlRequests[0][1], /^\d+$/);
assert.deepEqual(controlRequests[1].slice(0, 7), [
  "report_user_interaction_v1",
  controlRequests[1][1],
  "42",
  "任务-1",
  "approval-1",
  "requested",
  "1800000000000000002",
]);
// The reconciliation fallback must use the injected OpenCode transport. Normal
// CLI mode provides serverUrl as localhost:4096 without an HTTP listener.
const priorPollEnv = {
  enabled: process.env.ACTRAIL_IDLE_DETECTION_ENABLED,
  traceId: process.env.ACTRAIL_TRACE_ID,
  socketPath: process.env.ACTRAIL_CONTROL_SOCKET,
  taskId: process.env.ACTRAIL_OPENCODE_TASK_ID,
  interval: process.env.ACTRAIL_OPENCODE_PERMISSION_POLL_INTERVAL_MS,
};
process.env.ACTRAIL_IDLE_DETECTION_ENABLED = "true";
process.env.ACTRAIL_TRACE_ID = "42";
process.env.ACTRAIL_CONTROL_SOCKET = controlSocket;
process.env.ACTRAIL_OPENCODE_TASK_ID = "launch:42:poll-transport";
process.env.ACTRAIL_OPENCODE_PERMISSION_POLL_INTERVAL_MS = "100";
const pollRequests = [];
const originalFetch = globalThis.fetch;
globalThis.fetch = async () => { throw new Error("global fetch must not be used"); };
try {
  await pluginModule.default({
    client: {
      _client: {
        get: async ({ url, query }) => {
          pollRequests.push({ url, query });
          if (url === "/api/permission/request") {
            return { data: { data: [{ id: "per_transport", sessionID: "ses_transport", permission: "bash", patterns: ["pwd"] }] } };
          }
          return { data: [] };
        },
      },
      app: { log: async () => ({}) },
    },
    directory: "/workspace",
    serverUrl: new URL("http://localhost:4096"),
  });
  await new Promise((resolve) => setTimeout(resolve, 150));
} finally {
  globalThis.fetch = originalFetch;
  for (const [key, value] of Object.entries(priorPollEnv)) {
    if (value === undefined) delete process.env[{ enabled: "ACTRAIL_IDLE_DETECTION_ENABLED", traceId: "ACTRAIL_TRACE_ID", socketPath: "ACTRAIL_CONTROL_SOCKET", taskId: "ACTRAIL_OPENCODE_TASK_ID", interval: "ACTRAIL_OPENCODE_PERMISSION_POLL_INTERVAL_MS" }[key]];
    else process.env[{ enabled: "ACTRAIL_IDLE_DETECTION_ENABLED", traceId: "ACTRAIL_TRACE_ID", socketPath: "ACTRAIL_CONTROL_SOCKET", taskId: "ACTRAIL_OPENCODE_TASK_ID", interval: "ACTRAIL_OPENCODE_PERMISSION_POLL_INTERVAL_MS" }[key]] = value;
  }
}
pollRequests.sort((left, right) => left.url.localeCompare(right.url));
assert.deepEqual(pollRequests, [
  { url: "/api/permission/request", query: { location: { directory: "/workspace" } } },
  { url: "/permission", query: { directory: "/workspace" } },
]);

await new Promise((resolve, reject) => controlServer.close((error) => error ? reject(error) : resolve()));
await rm(controlTemp, { recursive: true, force: true });

console.log("opencode adapter fixture passed");
