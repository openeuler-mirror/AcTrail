// Translates OpenCode events into turn and interaction lifecycle reports.

import {
  INTERACTION_REASON,
  INTERACTION_STATE,
  OPENCODE_EVENT,
  TRANSITION_TYPE,
  TURN_STATE,
} from "./actrail-constants.js";
import { key, requestId, sessionId, userMessageId } from "./actrail-opencode-utils.js";

const DEFAULT_TASK_PREFIX = "opencode:";
const MAX_SEEN_USER_MESSAGES = 256;

// Build the stable interaction ID reported to AcTrail.
function interactionId(session, request) {
  return session ? `opencode:${session}:${request}` : `opencode:${request}`;
}

export function createOpenCodeInteractionAdapter({ taskId, report }) {
  const taskPrefix = taskId || DEFAULT_TASK_PREFIX;
  const sessions = new Map();
  let reportQueue = Promise.resolve();

  function enqueue(transition) {
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
        nextTurn: 0,
        taskId: undefined,
        started: false,
        startPromise: undefined,
        completionPromise: undefined,
        pending: new Map(),
        seenUserMessageIds: new Set(),
      };
      sessions.set(session, state);
    }
    return state;
  }

  function turnTaskId(session, state) {
    return `${taskPrefix}:session:${session}:turn:${state.nextTurn}`;
  }

  async function startTurn(session) {
    const state = sessionState(session);
    if (!state || state.started) return true;
    if (state.startPromise) return state.startPromise;
    state.nextTurn += 1;
    const candidateTaskId = turnTaskId(session, state);
    state.taskId = candidateTaskId;
    state.startPromise = enqueue({
      type: TRANSITION_TYPE.TURN,
      taskId: candidateTaskId,
      state: TURN_STATE.STARTED,
    }).then((reported) => {
      state.startPromise = undefined;
      if (reported) {
        state.started = true;
        return true;
      }
      state.taskId = undefined;
      return false;
    });
    return state.startPromise;
  }

  async function completeTurn(session) {
    const state = sessionState(session);
    if (!state) return true;
    if (state.completionPromise) return state.completionPromise;
    state.completionPromise = (async () => {
      if (state.startPromise && !state.started && !(await state.startPromise)) return false;
      if (!state.started || state.pending.size !== 0) return true;
      const completedTaskId = state.taskId;
      const reported = await enqueue({
        type: TRANSITION_TYPE.TURN,
        taskId: completedTaskId,
        state: TURN_STATE.COMPLETED,
      });
      if (!reported) return false;
      state.started = false;
      state.taskId = undefined;
      return true;
    })();
    const completion = state.completionPromise;
    return completion.finally(() => {
      if (state.completionPromise === completion) state.completionPromise = undefined;
    });
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
    if (!request) return true;
    const session = sessionId(properties);
    if (!session) return true;
    const state = sessionState(session);
    const pendingKey = key(session, request);
    if (state.pending.has(pendingKey)) return true;
    const value = {
      session,
      request,
      interactionId: interactionId(session, request),
      reason,
      resolving: undefined,
      requestPromise: undefined,
    };
    state.pending.set(pendingKey, value);
    value.requestPromise = (async () => {
      if (!(await startTurn(session))) {
        state.pending.delete(pendingKey);
        return false;
      }
      const reported = await enqueue({
        type: TRANSITION_TYPE.INTERACTION,
        taskId: state.taskId,
        interactionId: value.interactionId,
        reason,
        state: INTERACTION_STATE.REQUESTED,
      });
      if (!reported) state.pending.delete(pendingKey);
      return reported;
    })();
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
    if (value.requestPromise && !(await value.requestPromise)) return false;
    if (value.resolving === state) return true;
    const owner = sessionState(value.session);
    value.resolving = state;
    const reported = await enqueue({
      type: TRANSITION_TYPE.INTERACTION,
      taskId: owner.taskId,
      interactionId: value.interactionId,
      reason: value.reason,
      state,
    });
    if (reported) owner.pending.delete(pendingKey);
    else value.resolving = undefined;
    return reported;
  }

  async function deleteSession(session) {
    if (!session) return true;
    const state = sessionState(session);
    if (!state) return true;
    let cleanupFailed = false;
    for (const [pendingKey, value] of [...state.pending]) {
      if (value.requestPromise && !(await value.requestPromise)) {
        cleanupFailed = true;
        continue;
      }
      const reported = await enqueue({
        type: TRANSITION_TYPE.INTERACTION,
        taskId: state.taskId,
        interactionId: value.interactionId,
        reason: value.reason,
        state: INTERACTION_STATE.RESOLVED,
      });
      if (reported) state.pending.delete(pendingKey);
      else cleanupFailed = true;
    }
    if (cleanupFailed) return false;
    return completeTurn(session);
  }
  // Reconcile pending permissions when plugin events are incomplete.
  async function syncPermissions(permissions) {
    if (!Array.isArray(permissions)) return false;
    const listed = new Set();
    for (const permission of permissions) {
      const session = sessionId(permission);
      const request = requestId(permission);
      if (!session || !request) continue;
      listed.add(key(session, request));
      if (!(await requestPermission(permission))) return false;
    }
    for (const [session, state] of sessions) {
      for (const [pendingKey, value] of [...state.pending]) {
        if (value.reason !== INTERACTION_REASON.APPROVAL || listed.has(pendingKey)) continue;
        if (!(await finish({ sessionID: session, requestID: value.request }))) return false;
      }
    }
    return true;
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
        return deleteSession(sessionId(properties));
      case OPENCODE_EVENT.SESSION_IDLE:
      case OPENCODE_EVENT.SESSION_STATUS:
        if (type === OPENCODE_EVENT.SESSION_STATUS && !statusIsIdle(properties)) return true;
        {
          const session = sessionId(properties);
          const state = sessionState(session);
          if (!state || state.pending.size !== 0) return true;
          return completeTurn(session);
        }
      case OPENCODE_EVENT.MESSAGE_UPDATED:
        if (!isUserMessage(properties)) return true;
        {
          const session = sessionId(properties);
          const messageId = userMessageId(properties);
          if (hasUserMessage(session, messageId)) return true;
          return startTurn(session).then((reported) => {
            if (reported) rememberUserMessage(session, messageId);
            return reported;
          });
        }
      default:
        return true;
    }
  }

  return {
    handle,
    syncPermissions,
    pendingCount: () => [...sessions.values()].reduce((count, state) => count + state.pending.size, 0),
  };
}
