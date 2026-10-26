// Reports OpenCode lifecycle transitions to AcTrail over a Unix socket.

import { createConnection } from "node:net";
import {
  CONTROL_COMMAND,
  CONTROL_REPLY,
  TRANSITION_TYPE,
} from "./actrail-constants.js";

const MAX_U64 = (1n << 64n) - 1n;
const MAX_REPLY_BYTES = 64 * 1024;
const DEFAULT_TIMEOUT_MS = 5_000;

let lastRequestId = 0n;
// Parse an unsigned 64-bit integer.
function parseU64(value, name) {
  const text = String(value);
  if (!/^\d+$/.test(text)) {
    throw new Error(`${name} must be an unsigned integer`);
  }
  const parsed = BigInt(text);
  if (parsed > MAX_U64) {
    throw new Error(`${name} exceeds u64`);
  }
  return parsed;
}

function unixNanosNow() {
  return BigInt(Date.now()) * 1_000_000n;
}
// Generate a monotonic request ID within this process.
function nextRequestId(now) {
  const candidate = parseU64(now(), "request id clock");
  lastRequestId = candidate > lastRequestId ? candidate : lastRequestId + 1n;
  if (lastRequestId > MAX_U64) {
    throw new Error("request id exceeds u64");
  }
  return lastRequestId.toString();
}
// Encode fields as length-prefixed UTF-8 frames.
export function encodeControlFields(fields) {
  return Buffer.concat(fields.map((field) => {
    const value = Buffer.from(String(field), "utf8");
    return Buffer.concat([Buffer.from(`${value.length}#`, "ascii"), value]);
  }));
}
// Decode length-prefixed UTF-8 frames.
export function decodeControlFields(input) {
  const bytes = Buffer.from(input);
  const fields = [];
  let cursor = 0;
  while (cursor < bytes.length) {
    const separator = bytes.indexOf(0x23, cursor);
    if (separator < 0) throw new Error("control reply field is missing its length separator");
    const rawLength = bytes.subarray(cursor, separator).toString("ascii");
    if (!/^\d+$/.test(rawLength)) throw new Error("control reply field length is invalid");
    const length = Number(rawLength);
    if (!Number.isSafeInteger(length)) throw new Error("control reply field length is too large");
    const start = separator + 1;
    const end = start + length;
    if (end > bytes.length) throw new Error("control reply field is truncated");
    fields.push(bytes.subarray(start, end).toString("utf8"));
    cursor = end;
  }
  return fields;
}
// Normalize a trace ID to its numeric form.
function normalizeTraceId(traceId) {
  const raw = String(traceId).replace(/^trace-/, "");
  return parseU64(raw, "trace id").toString();
}
// Encode a transition as control request fields.
function transitionFields(transition, traceId, requestId, observedAt) {
  if (transition?.type === TRANSITION_TYPE.WORK) {
    return [CONTROL_COMMAND.REPORT_WORK_LIFECYCLE, requestId, traceId,
      transition.sessionId, transition.taskId, transition.kind, transition.state, observedAt];
  }
  if (transition?.type === TRANSITION_TYPE.TURN) {
    return [
      CONTROL_COMMAND.REPORT_TURN_LIFECYCLE,
      requestId,
      traceId,
      transition.sessionId,
      transition.taskId,
      transition.state,
      observedAt,
    ];
  }
  if (transition?.type === TRANSITION_TYPE.INTERACTION) {
    return [
      CONTROL_COMMAND.REPORT_USER_INTERACTION,
      requestId,
      traceId,
      transition.sessionId,
      transition.taskId,
      transition.interactionId,
      transition.state,
      observedAt,
    ];
  }
  if (transition?.type === TRANSITION_TYPE.SESSION_CLOSED) {
    return [CONTROL_COMMAND.REPORT_SESSION_CLOSED, requestId, traceId,
      transition.sessionId, observedAt];
  }
  throw new Error(`unsupported AcTrail transition type: ${transition?.type}`);
}
// Select the expected acknowledgement for a transition.
function expectedReply(transition) {
  if (transition.type === TRANSITION_TYPE.WORK) {
    return CONTROL_REPLY.WORK_LIFECYCLE_RECORDED;
  }
  if (transition.type === TRANSITION_TYPE.SESSION_CLOSED) {
    return CONTROL_REPLY.SESSION_CLOSED_RECORDED;
  }
  return transition.type === TRANSITION_TYPE.TURN
    ? CONTROL_REPLY.TURN_LIFECYCLE_RECORDED
    : CONTROL_REPLY.USER_INTERACTION_RECORDED;
}
// Send a framed request over a Unix socket.
export function sendControlRequest(
  socketPath,
  request,
  { timeoutMs = DEFAULT_TIMEOUT_MS } = {},
) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let replyBytes = 0;

    let settled = false;
    const settle = (callback, value) => {
      if (settled) return;
      settled = true;
      callback(value);
    };
    const socket = createConnection({
      path: socketPath,
      allowHalfOpen: true,
    });

    socket.setTimeout(timeoutMs, () => {
      socket.destroy(
        new Error(`AcTrail control request timed out after ${timeoutMs}ms`),
      );
    });

    socket.on("connect", () => {
      socket.write(request, (error) => {
        if (error) {
          socket.destroy(error);
        }
      });
    });

    socket.on("data", (chunk) => {
      replyBytes += chunk.length;
      if (replyBytes > MAX_REPLY_BYTES) {
        socket.destroy(
          new Error("AcTrail control reply exceeds the configured byte limit"),
        );
        return;
      }
      chunks.push(chunk);
    });

    socket.on("end", () => {
      settle(resolve, Buffer.concat(chunks));
    });

    socket.on("error", (error) => {
      settle(reject, error);
    });
  });
}


export function createControlSocketReporter({
  traceId,
  socketPath,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  now = unixNanosNow,
  observedAtUnixNanos,
}) {
  const normalizedTraceId = normalizeTraceId(traceId);
  if (!socketPath) throw new Error("AcTrail control socket path is required");
  const fixedObservedAt = observedAtUnixNanos === undefined || observedAtUnixNanos === ""
    ? undefined
    : parseU64(observedAtUnixNanos, "observed-at-unix-nanos").toString();

  // Generate IDs, send the transition, and validate the reply.
  return async function reportTransition(transition) {
    const observedAt = fixedObservedAt ?? transition.observedAtUnixNanos
      ?? parseU64(now(), "observed-at-unix-nanos").toString();
    const requestId = nextRequestId(now);
    const request = encodeControlFields(
      transitionFields(transition, normalizedTraceId, requestId, observedAt),
    );
    const reply = decodeControlFields(
      await sendControlRequest(socketPath, request, { timeoutMs }),
    );
    if (reply[0] === CONTROL_REPLY.ERROR) {
      throw new Error(`${reply[1] || "control_error"}: ${reply[2] || "unknown error"}`);
    }
    const expected = expectedReply(transition);
    if (reply.length !== 1 || reply[0] !== expected) {
      throw new Error(`unexpected AcTrail control reply: ${reply[0] || "empty"}`);
    }
  };
}
