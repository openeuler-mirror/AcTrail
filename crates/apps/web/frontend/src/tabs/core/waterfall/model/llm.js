import { buildLlmMessages, llmRequestMessage, llmResponseMessage, previewText } from '../../../../llm/insight.js';
import { toBigInt, nanosDiffMs } from './utils.js';

export function attachLlmCallDetails(node, nodeById, parentByChild, window) {
  const requestAction =
    node.children.find((child) => child.kind === 'llm.request')?.action ??
    actionById(nodeById, node.action.attributes?.['llm.call.request_action_id']);
  const responseAction =
    node.children.find((child) => child.kind === 'llm.response')?.action ??
    actionById(nodeById, node.action.attributes?.['llm.call.response_action_id']);
  node.llmRequestAction = requestAction;
  node.llmResponseAction = responseAction;
  node.llmPhases = buildLlmPhases(requestAction, responseAction, window);
  node.llmContext = resolveLlmCallContext(node, nodeById, parentByChild);
}

export function ensureLlmMessages(node) {
  if (Object.prototype.hasOwnProperty.call(node, 'llmMessages')) {
    return node.llmMessages;
  }
  let messages = null;
  if (node.kind === 'llm.call') {
    messages = buildLlmMessages(node.llmRequestAction, node.llmResponseAction);
  } else if (node.kind === 'llm.request') {
    const requestFull = llmRequestMessage(node.action);
    messages = buildLlmMessages(node.action, null, requestFull, '');
  } else if (node.kind === 'llm.response') {
    const responseFull = llmResponseMessage(node.action);
    messages = buildLlmMessages(null, node.action, '', responseFull);
  }
  node.llmMessages = messages;
  return messages;
}

function buildLlmPhases(requestAction, responseAction, window) {
  const request = requestAction ? phaseFromAction(requestAction, window) : null;
  const response = responseAction ? phaseFromAction(responseAction, window) : null;
  if (!request && !response) {
    return null;
  }
  let gap = null;
  if (request && response && request.durMs !== null) {
    const requestEndMs = request.startOffsetMs + request.durMs;
    if (response.startOffsetMs > requestEndMs) {
      gap = {
        startOffsetMs: requestEndMs,
        durMs: response.startOffsetMs - requestEndMs,
      };
    }
  }
  return { request, response, gap };
}

function phaseFromAction(action, window) {
  const startNanos = toBigInt(action.start_time_unix_nanos);
  const endNanos = action.end_time_unix_nanos ? toBigInt(action.end_time_unix_nanos) : null;
  return {
    startOffsetMs: nanosDiffMs(startNanos, window.startNanos),
    durMs: endNanos === null ? null : nanosDiffMs(endNanos, startNanos),
    live: endNanos === null,
  };
}

function resolveLlmCallContext(node, nodeById, parentByChild) {
  const pid = node.action.process?.pid;
  const ancestors = walkAncestorNodes(node.id, nodeById, parentByChild);
  const commandAncestors = ancestors.filter(
    (ancestor) => ancestor.kind === 'command.invocation' || ancestor.kind === 'agent.invocation',
  );
  const parentCommand = commandAncestors[0] ?? null;
  const trigger = parentCommand?.action.attributes?.['agent.invocation.trigger'];
  const invocationKind = parentCommand?.action.attributes?.['invocation.kind'];
  let scope = 'primary';
  if (trigger === 'child_llm_request') {
    scope = 'subagent';
  } else if (commandAncestors.length > 1) {
    scope = 'nested';
  } else if (invocationKind === 'agent' && commandAncestors.length) {
    scope = 'agent';
  }
  const parentLabel = parentCommand ? commandContextLabel(parentCommand.action) : '';
  return {
    scope,
    pid,
    parentLabel,
    scopeLabel: llmScopeLabel(scope, pid),
  };
}

function walkAncestorNodes(nodeId, nodeById, parentByChild) {
  const ancestors = [];
  const seen = new Set();
  let currentId = parentByChild.get(nodeId);
  while (currentId && !seen.has(currentId)) {
    seen.add(currentId);
    const ancestor = nodeById.get(currentId);
    if (!ancestor) {
      break;
    }
    ancestors.push(ancestor);
    currentId = parentByChild.get(currentId);
  }
  return ancestors;
}

function commandContextLabel(action) {
  const attrs = action.attributes ?? {};
  return previewText(
    attrs['agent.child.command_line'] ?? attrs['command.line'] ?? action.title ?? '',
    72,
  );
}

function llmScopeLabel(scope, pid) {
  const pidLabel = pid === undefined || pid === null ? '' : `pid ${pid}`;
  switch (scope) {
    case 'subagent':
      return pidLabel ? `subagent · ${pidLabel}` : 'subagent';
    case 'nested':
      return pidLabel ? `nested agent · ${pidLabel}` : 'nested agent';
    case 'agent':
      return pidLabel ? `agent · ${pidLabel}` : 'agent';
    default:
      return pidLabel || '';
  }
}

export function llmMessagesFromAction(action) {
  if (action?.kind !== 'llm.call') {
    if (action?.kind === 'llm.request') {
      const requestFull = llmRequestMessage(action);
      return buildLlmMessages(action, null, requestFull, '');
    }
    if (action?.kind === 'llm.response') {
      const responseFull = llmResponseMessage(action);
      return buildLlmMessages(null, action, '', responseFull);
    }
    return null;
  }
  return null;
}

function actionById(nodeById, actionId) {
  if (!actionId) {
    return null;
  }
  return nodeById.get(actionId)?.action ?? null;
}
