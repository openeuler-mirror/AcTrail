// Translates OpenCode events into turn and interaction lifecycle reports.

import {
  INTERACTION_REASON,
  INTERACTION_STATE,
  OPENCODE_EVENT,
  TRANSITION_TYPE,
  TURN_STATE,
} from "./actrail-constants.js";
import { key, requestId, sessionId, userMessageId } from "./actrail-opencode-utils.js";

const MAX_SEEN_USER_MESSAGES = 256;

// Build the stable interaction ID reported to AcTrail.
function interactionId(session, request) {
  return session ? `opencode:${session}:${request}` : `opencode:${request}`;
}

export function createOpenCodeInteractionAdapter({ report }) {
  const sessions = new Map();
  let reportQueue = Promise.resolve();

  function enqueue(transition) {
    transition.observedAtUnixNanos ??= (BigInt(Date.now()) * 1_000_000n).toString();
    const next = reportQueue
      .catch(() => false)
      .then(async () => {
        try {
          const reported = await report(transition);
          return reported !== false;
        } catch {
          return false;
        }
      });
    reportQueue = next;
    return next;
  }

  function sessionState(session) {
    if (!session) return undefined;
    let state = sessions.get(session);
    if (!state) {
      state = {
        taskId: undefined,
        started: false,
        modelPending: false,
        activeTools: new Set(),
        pending: new Map(),
        seenUserMessageIds: new Set(),
      };
      sessions.set(session, state);
    }
    return state;
  }

  function startTurn(session, messageId) {
    const state = sessionState(session);
    if (!state || !messageId) return Promise.resolve(false);
    if (state.started && state.taskId === messageId) return Promise.resolve(true);
    if (state.started) void completeTurn(session);
    // Native lifecycle state is independent of whether AcTrail accepted its report.
    state.taskId = messageId;
    state.started = true;
    state.modelPending = false;
    state.activeTools.clear();
    return enqueue({ type: TRANSITION_TYPE.TURN, sessionId: session,
      taskId: messageId, state: TURN_STATE.STARTED });
  }

  function completeTurn(session) {
    const state = sessionState(session);
    if (!state || !state.started) return Promise.resolve(true);
    const completedTaskId = state.taskId;
    state.started = false;
    state.taskId = undefined;
    state.modelPending = false;
    state.activeTools.clear();
    state.pending.clear();
    return enqueue({ type: TRANSITION_TYPE.TURN, sessionId: session,
      taskId: completedTaskId, state: TURN_STATE.COMPLETED });
  }
  // Deduplicate user-message lifecycle events.
  function rememberUserMessage(session, messageId) {
    if (!messageId) return false;
    const state = sessionState(session);
    if (!state) return false;
    if (state.seenUserMessageIds.has(messageId)) return true;
    state.seenUserMessageIds.add(messageId);
    while (state.seenUserMessageIds.size > MAX_SEEN_USER_MESSAGES) {
      state.seenUserMessageIds.delete(state.seenUserMessageIds.values().next().value);
    }
    return false;
  }
  // Check whether a user message was already handled.
  function hasUserMessage(session, messageId) {
    if (!messageId) return false;
    return sessionState(session)?.seenUserMessageIds.has(messageId) ?? false;
  }
  async function requestPermission(properties, reason = INTERACTION_REASON.APPROVAL) {
    const request = requestId(properties);
    const session = sessionId(properties);
    if (!request || !session) return true;
    const state = sessionState(session);
    const pendingKey = key(session, request);
    let value = state.pending.get(pendingKey);
    if (!value) {
      value = { session, request, interactionId: interactionId(session, request), reason,
        observedAtUnixNanos: (BigInt(Date.now()) * 1_000_000n).toString(),
        requestPromise: undefined, resolving: undefined, taskId: undefined };
      state.pending.set(pendingKey, value);
    }
    if (value.requestPromise) return value.requestPromise;
    // A permission is not proof of a new user turn. Wait for its native user message.
    if (!state.started) return false;
    value.taskId = state.taskId;
    value.requestPromise = enqueue({
      type: TRANSITION_TYPE.INTERACTION,
      sessionId: session,
      taskId: value.taskId,
      interactionId: value.interactionId,
      reason: value.reason,
      state: INTERACTION_STATE.REQUESTED,
      observedAtUnixNanos: value.observedAtUnixNanos,
    });
    // A lost acknowledgement cannot establish that a counter increment was lost.
    // Report each native interaction once; retries could increment it twice.
    return value.requestPromise;
  }

  async function finish(properties, state = INTERACTION_STATE.RESOLVED) {
    const request = requestId(properties);
    if (!request) return true;
    const session = sessionId(properties);
    const sessionStateValue = sessionState(session);
    let pendingKey = key(session, request);
    let value = sessionStateValue?.pending.get(pendingKey);
    if (!value && !session) {
      const matches = [...sessions.values()]
        .flatMap((candidate) => [...candidate.pending.entries()])
        .filter(([, item]) => item.request === request);
      if (matches.length === 1) {
        pendingKey = matches[0][0];
        value = matches[0][1];
      }
    }
    if (!value) return true;
    if (!value.taskId) {
      sessionState(value.session).pending.delete(pendingKey);
      return true;
    }
    if (value.resolving === state) return true;
    const owner = sessionState(value.session);
    value.resolving = state;
    owner.pending.delete(pendingKey);
    const reported = await enqueue({
      type: TRANSITION_TYPE.INTERACTION,
      sessionId: value.session,
      taskId: value.taskId,
      interactionId: value.interactionId,
      reason: value.reason,
      state,
    });
    owner.pending.delete(pendingKey);
    return reported;
  }

  async function deleteSession(session) {
    if (!session) return true;
    sessions.delete(session);
    return enqueue({ type: TRANSITION_TYPE.SESSION_CLOSED, sessionId: session });
  }

  function workTransition(session, state, kind, started) {
    return enqueue({ type: TRANSITION_TYPE.WORK, sessionId: session,
      taskId: state.taskId, kind, state: started ? "started" : "completed" });
  }

  function modelStarted(input) {
    // OpenCode title generation runs independently of the task's model step.
    if (input.agent === "title") return Promise.resolve(true);
    const session = input.sessionID;
    // The hook's message is OpenCode's selected UserMessage for this model step.
    // Queued user-message events alone do not replace the executing task.
    const message = input.message;
    if (message?.role !== "user" || message.sessionID !== session || !message.id) {
      return Promise.resolve(true);
    }
    void startTurn(session, message.id);
    const state = sessions.get(session);
    if (!state?.started || state.modelPending) return Promise.resolve(true);
    // A retry repeats chat.headers without a successful step-finish.
    state.modelPending = true;
    return workTransition(session, state, "model", true);
  }

  function workPartUpdated(part) {
    const session = part?.sessionID;
    const state = sessions.get(session);
    if (!state?.started) return Promise.resolve(true);
    if (part.type === "step-finish" && state.modelPending) {
      state.modelPending = false;
      return workTransition(session, state, "model", false);
    }
    if (part.type !== "tool" || !part.callID) return Promise.resolve(true);
    if (part.state?.status === "running") {
      if (state.activeTools.has(part.callID)) return Promise.resolve(true);
      state.activeTools.add(part.callID);
      return workTransition(session, state, "tool", true);
    }
    if ((part.state?.status === "completed" || part.state?.status === "error")
        && state.activeTools.delete(part.callID)) {
      return workTransition(session, state, "tool", false);
    }
    return Promise.resolve(true);
  }

  async function syncPermissions(permissions) {
    if (!Array.isArray(permissions)) return false;
    const listed = new Set();
    let reported = true;
    for (const permission of permissions) {
      const session = sessionId(permission);
      const request = requestId(permission);
      if (!session || !request) continue;
      listed.add(key(session, request));
      if (!(await requestPermission(permission))) reported = false;
    }
    for (const [session, state] of sessions) {
      for (const [pendingKey, value] of [...state.pending]) {
        if (value.reason !== INTERACTION_REASON.APPROVAL || listed.has(pendingKey)) continue;
        if (!(await finish({ sessionID: session, requestID: value.request }))) reported = false;
      }
    }
    return reported;
  }

  function statusIsIdle(properties) {
    const status = properties?.status;
    return status === "idle"
      || status?.type === "idle"
      || status?.status === "idle";
  }

  function isUserMessage(properties) {
    const message = properties?.info || properties?.message || properties;
    return message?.role === "user";
  }

  async function handle(event) {
    const type = event?.type;
    const properties = event?.properties || event?.data || {};
    switch (type) {
      case OPENCODE_EVENT.MESSAGE_PART_UPDATED:
        return workPartUpdated(properties.part);
      case "session.error":
        return completeTurn(sessionId(properties));
      case OPENCODE_EVENT.PERMISSION_ASKED:
      case OPENCODE_EVENT.PERMISSION_V2_ASKED:
        return requestPermission(properties);
      case OPENCODE_EVENT.PERMISSION_REPLIED:
      case OPENCODE_EVENT.PERMISSION_V2_REPLIED:
        return finish(properties);
      case OPENCODE_EVENT.QUESTION_ASKED: // Wait for user input.
      case OPENCODE_EVENT.QUESTION_V2_ASKED:
        return requestPermission(properties, INTERACTION_REASON.INPUT);
      case OPENCODE_EVENT.QUESTION_REPLIED:
      case OPENCODE_EVENT.QUESTION_REJECTED:
      case OPENCODE_EVENT.QUESTION_V2_REPLIED:
      case OPENCODE_EVENT.QUESTION_V2_REJECTED:
        return finish(properties);
      case OPENCODE_EVENT.SESSION_DELETED:
        return deleteSession(sessionId(properties) || properties?.info?.id);
      case OPENCODE_EVENT.SESSION_IDLE:
      case OPENCODE_EVENT.SESSION_STATUS:
        if (type === OPENCODE_EVENT.SESSION_STATUS && !statusIsIdle(properties)) return true;
        {
          const session = sessionId(properties);
          const state = sessionState(session);
          if (!state) return true;
          return completeTurn(session);
        }
      case OPENCODE_EVENT.MESSAGE_UPDATED:
        if (!isUserMessage(properties)) return true;
        {
          const session = sessionId(properties);
          const messageId = userMessageId(properties);
          if (hasUserMessage(session, messageId)) return true;
          if (sessions.get(session)?.started) {
            rememberUserMessage(session, messageId);
            return true;
          }
          const startedReport = startTurn(session, messageId);
          rememberUserMessage(session, messageId);
          const state = sessionState(session);
          const pendingReports = [...state.pending.values()].map((value) =>
            requestPermission({ sessionID: session, requestID: value.request }, value.reason));
          return Promise.all([startedReport, ...pendingReports]).then((results) => results.every(Boolean));
        }
      default:
        return true;
    }
  }

  return {
    handle,
    modelStarted,
    syncPermissions,
    pendingCount: () => [...sessions.values()].reduce((count, state) => count + state.pending.size, 0),
  };
}
