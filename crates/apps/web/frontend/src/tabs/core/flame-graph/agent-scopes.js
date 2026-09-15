import { toolCallArguments } from './request-context.js';

// Supplement missing stored links for this view only. Require an exact text block
// and a unique pairing in the tool execution window; timing alone is insufficient.
export function inferAgentRequestLinks(actions, links, actionById, displayIntervals, requestContexts) {
  const validLinks = links.filter((link) => link.valid !== false);
  const linkedInvocations = new Set();
  const linkedRequests = new Set();
  const continuingRequests = new Set();
  const toolsByInvocation = new Map();
  for (const link of validLinks) {
    if (link.role === 'agent.invocation.child_llm_request') {
      linkedInvocations.add(link.parent);
      linkedRequests.add(link.child);
    } else if (link.role === 'llm.request.trajectory_parent') {
      continuingRequests.add(link.child);
    } else if (link.role === 'llm.tool_call.agent_invocation') {
      toolsByInvocation.set(link.child, link.parent);
    }
  }
  const requests = actions.filter((action) => {
    if (action.kind !== 'llm.request' || linkedRequests.has(action.id) || continuingRequests.has(action.id)) return false;
    const trajectory = action.attributes?.['llm.request.trajectory_id'];
    return (!trajectory || trajectory === action.id)
      && requestContexts.has(action.id) && !requestContexts.get(action.id).backgroundKind;
  });
  const matches = [];
  const requestMatchCounts = new Map();
  for (const invocation of actions) {
    if (invocation.kind !== 'agent.invocation' || linkedInvocations.has(invocation.id)) continue;
    const toolId = invocation.attributes?.['agent.invocation.tool_call_action_id'] ?? toolsByInvocation.get(invocation.id);
    const tool = actionById.get(toolId);
    const prompt = toolCallArguments(tool, actionById)?.prompt;
    if (typeof prompt !== 'string' || !prompt.trim()) continue;
    const interval = displayIntervals.get(toolId) ?? {
      startNanos: BigInt(invocation.start_time_unix_nanos),
      endNanos: invocation.end_time_unix_nanos ? BigInt(invocation.end_time_unix_nanos) : null,
    };
    const candidates = requests.filter((request) => {
      const start = BigInt(request.start_time_unix_nanos);
      return start >= interval.startNanos && (interval.endNanos == null || start <= interval.endNanos)
        && requestContexts.get(request.id).userTexts.some((text) => text.trim() === prompt.trim());
    });
    // Count every possible pairing, including invocations with several candidates.
    for (const request of candidates) {
      requestMatchCounts.set(request.id, (requestMatchCounts.get(request.id) ?? 0) + 1);
    }
    if (candidates.length === 1) matches.push({ invocation, request: candidates[0] });
  }
  return matches.filter(({ request }) => requestMatchCounts.get(request.id) === 1)
    .map(({ invocation, request }) => ({
      parent: invocation.id, child: request.id,
      role: 'agent.invocation.child_llm_request', valid: true, inferred: true,
    }));
}

export function deriveAgentScopes(invocations, actions, links, childrenByParent) {
  const validLinks = (links ?? []).filter((link) => link.valid !== false);
  const childRequestsByInvocation = new Map();
  const parentToolCallByInvocation = new Map();
  const callIdsByRequest = new Map();

  for (const link of validLinks) {
    if (link.role === 'agent.invocation.child_llm_request') {
      appendMapValue(childRequestsByInvocation, link.parent, link.child);
    } else if (link.role === 'llm.tool_call.agent_invocation') {
      parentToolCallByInvocation.set(link.child, link.parent);
    } else if (link.role === 'llm.call.request') {
      appendMapValue(callIdsByRequest, link.child, link.parent);
    }
  }
  for (const action of actions) {
    if (action.kind !== 'llm.call') {
      continue;
    }
    const requestId = action.attributes?.['llm.call.request_action_id'];
    if (requestId) {
      appendMapValue(callIdsByRequest, requestId, action.id);
    }
  }

  const scopes = invocations.map((activity, index) => {
    const invocation = activity.action;
    const actionIds = new Set();
    collectDescendants(invocation.id, childrenByParent, actionIds);
    for (const requestId of childRequestsByInvocation.get(invocation.id) ?? []) {
      collectDescendants(requestId, childrenByParent, actionIds);
    }
    expandAgentCalls(actionIds, callIdsByRequest, childrenByParent);

    const attributes = invocation.attributes ?? {};
    const parentToolCallId = attributes['agent.invocation.tool_call_action_id']
      ?? parentToolCallByInvocation.get(invocation.id)
      ?? null;
    const agentType = invocationAgentType(invocation, index + 1);
    const toolName = attributes['agent.invocation.tool_name'];
    const label = agentType.startsWith('Subagent ')
      ? agentType
      : `${agentType} agent`;
    return {
      id: `agent-subagent-${invocation.id}`,
      role: 'subagent',
      label,
      agentType,
      invocation,
      parentToolCallId,
      relationSource: validLinks.some((link) => link.parent === invocation.id
        && link.role === 'agent.invocation.child_llm_request' && link.inferred) ? 'inferred from request text' : 'stored relationship',
      actionIds,
      parentId: 'agent-main',
      parentLabel: 'Main agent',
      depth: 1,
      childCount: 0,
      spawnLabel: toolName ? `tool.call:${toolName}` : 'Agent invocation',
      startOffsetMs: activity.startOffsetMs,
      searchText: ['subagent', agentType, toolName, invocation.title]
        .filter(Boolean)
        .join(' ')
        .toLowerCase(),
    };
  });

  for (const scope of scopes) {
    const parent = scopes
      .filter((candidate) => (
        candidate.id !== scope.id
        && scope.parentToolCallId
        && candidate.actionIds.has(scope.parentToolCallId)
      ))
      .sort((left, right) => left.actionIds.size - right.actionIds.size)[0];
    if (parent) {
      scope.parentId = parent.id;
      scope.parentLabel = parent.label;
    }
  }

  const byId = new Map(scopes.map((scope) => [scope.id, scope]));
  for (const scope of scopes) {
    scope.depth = agentScopeDepth(scope, byId);
    scope.childCount = scopes.filter((candidate) => candidate.parentId === scope.id).length;
  }
  return scopes.sort(
    (left, right) => left.depth - right.depth || left.startOffsetMs - right.startOffsetMs,
  );
}

export function orderAgentScopes(definitions) {
  const childrenByScope = new Map();
  for (const scope of definitions.slice(1)) {
    appendMapValue(childrenByScope, scope.parentId ?? 'agent-main', scope);
  }
  for (const children of childrenByScope.values()) {
    children.sort((left, right) => left.startOffsetMs - right.startOffsetMs);
  }
  const ordered = [];
  const visit = (scope) => {
    ordered.push(scope);
    for (const child of childrenByScope.get(scope.id) ?? []) {
      visit(child);
    }
  };
  visit(definitions[0]);
  return ordered;
}

function expandAgentCalls(actionIds, callIdsByRequest, childrenByParent) {
  let changed = true;
  while (changed) {
    changed = false;
    for (const requestId of [...actionIds]) {
      for (const callId of callIdsByRequest.get(requestId) ?? []) {
        if (!actionIds.has(callId)) {
          collectDescendants(callId, childrenByParent, actionIds);
          changed = true;
        }
      }
    }
  }
}

function collectDescendants(rootId, childrenByParent, destination) {
  const pending = [rootId];
  while (pending.length) {
    const id = pending.pop();
    if (!id || destination.has(id)) {
      continue;
    }
    destination.add(id);
    pending.push(...(childrenByParent.get(id) ?? []));
  }
}

function agentScopeDepth(scope, byId, visited = new Set()) {
  if (!scope.parentId || scope.parentId === 'agent-main' || visited.has(scope.id)) {
    return 1;
  }
  visited.add(scope.id);
  return 1 + agentScopeDepth(byId.get(scope.parentId) ?? {}, byId, visited);
}

function invocationAgentType(invocation, ordinal) {
  const declared = String(
    invocation.attributes?.['agent.invocation.agent_type'] ?? '',
  ).trim();
  if (declared) {
    return humanize(declared);
  }
  const titleMatch = String(invocation.title ?? '').match(/^Invoke\s+(.+?)\s+agent$/i);
  return titleMatch?.[1] ? humanize(titleMatch[1]) : `Subagent ${ordinal}`;
}

function appendMapValue(map, key, value) {
  if (!map.has(key)) {
    map.set(key, []);
  }
  const values = map.get(key);
  if (!values.includes(value)) {
    values.push(value);
  }
}

function humanize(value) {
  return String(value)
    .replace(/[._-]+/g, ' ')
    .replace(/^./, (letter) => letter.toUpperCase());
}
