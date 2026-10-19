export function deriveAgentScopes(invocations, actions, links, childrenByParent, t) {
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
        && link.role === 'agent.invocation.child_llm_request' && link.inferred)
        ? t('scope.inferredSource')
        : t('scope.storedSource'),
      actionIds,
      parentId: 'agent-main',
      parentLabel: t('partialOwnership'),
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
