import { semanticActionLabel, semanticActionTarget } from '../../actionLabels.js';
import {
  actionDetail,
  formatOffset,
} from '../waterfall/model.js';
import { deriveAgentScopes, inferAgentRequestLinks, orderAgentScopes } from './agent-scopes.js';
import { toolCallArguments } from './request-context.js';

const BACKGROUND_LABELS = Object.freeze({
  title_generation: 'Title generation',
  conversation_summary: 'Conversation summary',
  context_compaction: 'Context compaction',
  auxiliary_inferred: 'Background LLM (inferred)',
});

const AGENT_GRAPH_ROLES = new Set([
  'agent.invocation.exec',
  'agent.invocation.child_llm_request',
  'command.contains_file_access',
  'command.contains_command_invocation',
  'command.contains_llm_call',
  'command.contains_mcp_tool_call',
  'command.contains_process_exec',
  'command.contains_process_fork_attempt',
  'file.write.contains_file_event',
  'llm.call.request',
  'llm.call.response',
  'llm.response.tool_call',
  'llm.tool_call.result',
  'llm.tool_call.agent_invocation',
  'llm.request.llm_response',
  'llm.request.trajectory_parent',
  'llm.request.trajectory_fork',
  'mcp.tool_call.request',
  'mcp.tool_call.response',
  'mcp.request.stdout',
  'mcp.response.stdin',
]);

const FLAME_GRAPH_ASSOCIATION_ROLES = new Set([
  'llm.response.tool_call',
  'llm.tool_call.result',
  'agent.invocation.child_llm_request',
  'llm.tool_call.agent_invocation',
  'llm.request.trajectory_parent',
  'llm.request.trajectory_fork',
]);

const STRUCTURAL_KINDS = new Set([
  'agent.identity',
  'agent.exit',
]);

const BACKGROUND_LLM_KINDS = new Set([
  'llm.call',
  'llm.request',
  'llm.response',
]);

const AGENT_GROUP_ORDER = Object.freeze(['model', 'dialogue', 'tools', 'detail']);
const HARNESS_GROUP_ORDER = Object.freeze([
  'model',
  'dialogue',
  'tools',
  'commands',
  'filesystem',
  'process',
  'runtime',
  'protocol',
]);

const GROUP_LABELS = Object.freeze({
  model: 'LLM calls',
  dialogue: 'Messages',
  tools: 'Tool calls',
  detail: 'Tool effects',
  commands: 'Harness command',
  filesystem: 'Background file I/O',
  process: 'Process lifecycle',
  runtime: 'Runtime',
  protocol: 'Network / protocol',
});

const GROUP_DESCRIPTIONS = Object.freeze({
  model: 'Model request lifecycle',
  dialogue: 'Request and assistant response',
  tools: 'Declared tools, results, and child agents',
  detail: 'File effects linked to Agent work',
  commands: 'Top-level harness execution',
  filesystem: 'Framework file activity without an Agent tool link',
  process: 'fork, vfork, clone, exec, and exit observations',
  runtime: 'Enforcement and uncategorized runtime activity',
  protocol: 'HTTP, SSE, and MCP transport details',
});

export function buildFlameGraph(actions, links, attribution = null, associations = [], requestContexts = new Map()) {
  const validActions = (actions ?? []).filter(validTimedAction);
  if (!validActions.length) {
    return emptyFlameGraphModel();
  }

  const graphLinks = flameGraphLinks(links, associations);
  const window = computeWindow(validActions);
  const actionById = new Map(validActions.map((action) => [action.id, action]));
  const displayIntervals = derivedDisplayIntervals(validActions, graphLinks, actionById);
  graphLinks.push(...inferAgentRequestLinks(validActions, graphLinks, actionById, displayIntervals, requestContexts));
  const childrenByParent = graphChildren(graphLinks, actionById);
  const childRequests = new Set(graphLinks
    .filter((link) => link.valid !== false && link.role === 'agent.invocation.child_llm_request')
    .map((link) => link.child));
  const requestByCall = new Map();
  const backgroundKindByCall = new Map();

  for (const action of validActions) {
    if (action.kind !== 'llm.call') {
      continue;
    }
    const request = resolveCallRequest(action, actionById, childrenByParent);
    requestByCall.set(action.id, request ?? null);
    const backgroundKind = requestContexts.has(request?.id)
      ? requestContexts.get(request.id).backgroundKind
      : request?.attributes?.['llm.request.background_kind'];
    if (backgroundKind && !childRequests.has(request?.id)) {
      backgroundKindByCall.set(action.id, backgroundKind);
    }
  }
  inferAuxiliaryBackgroundCalls(validActions, requestByCall, backgroundKindByCall, graphLinks, requestContexts);
  const backgroundKindByAction = backgroundKindsByAction(
    backgroundKindByCall,
    childrenByParent,
    actionById,
  );
  const callOrdinalById = new Map(
    validActions
      .filter((action) => action.kind === 'llm.call' && !backgroundKindByCall.has(action.id))
      .sort(compareTimedActions)
      .map((action, index) => [action.id, index + 1]),
  );

  const agentActionIds = new Set();
  for (const action of validActions) {
    if (action.kind === 'llm.call' && !backgroundKindByCall.has(action.id)) {
      collectDescendants(action.id, childrenByParent, agentActionIds);
    }
  }
  const toolEffectByAction = attributedToolEffects(
    attribution,
    validActions,
    actionById,
    childrenByParent,
    displayIntervals,
    agentActionIds,
  );
  for (const actionId of toolEffectByAction.keys()) {
    agentActionIds.add(actionId);
  }

  const agentActivities = [];
  const harnessActivities = [];
  for (const action of validActions) {
    if (STRUCTURAL_KINDS.has(action.kind)) {
      continue;
    }
    const backgroundKind = backgroundKindByAction.get(action.id) ?? null;
    const layer = backgroundKind ? 'harness' : (agentActionIds.has(action.id) ? 'agent' : 'harness');
    const activity = activityFromAction(
      action,
      window,
      layer,
      requestByCall.get(action.id) ?? null,
      backgroundKind,
      callOrdinalById.get(action.id) ?? null,
      attributedDisplayInterval(
        displayIntervals.get(action.id) ?? null,
        toolEffectByAction.get(action.id) ?? null,
      ),
      toolEffectByAction.get(action.id) ?? null,
    );
    if (!activity) {
      continue;
    }
    (layer === 'agent' ? agentActivities : harnessActivities).push(activity);
  }

  const layers = [
    buildAgentLayer(
      agentActivities,
      validActions,
      graphLinks,
      childrenByParent,
      window,
    ),
    buildLayer('harness', 'Harness layer', 'Title/summary work and unclaimed framework activity', harnessActivities, window),
  ];

  return {
    window,
    layers,
    totalActivities: layers.reduce((total, layer) => total + layer.activityCount, 0),
  };
}

function flameGraphLinks(links, associations) {
  const merged = [...(links ?? [])];
  const seen = new Set(merged.map(linkIdentity));
  for (const link of associations ?? []) {
    if (!FLAME_GRAPH_ASSOCIATION_ROLES.has(link.role)) {
      continue;
    }
    const identity = linkIdentity(link);
    if (!seen.has(identity)) {
      merged.push(link);
      seen.add(identity);
    }
  }
  return merged;
}

function linkIdentity(link) {
  return `${link.parent}\u0000${link.child}\u0000${link.role}`;
}

export function emptyFlameGraphModel() {
  return {
    window: {
      startNanos: 0n,
      endNanos: 0n,
      spanMs: 1,
      startIso: null,
      endIso: null,
    },
    layers: [
      emptyLayer('agent', 'Agent layer', 'Primary LLM turns and their linked tool chain'),
      emptyLayer('harness', 'Harness layer', 'Title/summary work and unclaimed framework activity'),
    ],
    totalActivities: 0,
  };
}

export function filterFlameGraph(model, query) {
  const normalized = String(query ?? '').trim().toLowerCase();
  if (!normalized) {
    return model.layers;
  }
  return model.layers.map((layer) => {
    if (!layer.agentScopes?.length) {
      return { ...layer, tracks: filterTracks(layer.tracks, normalized) };
    }
    const agentScopes = layer.agentScopes
      .map((scope) => {
        if ((scope.searchText ?? '').includes(normalized)) {
          return scope;
        }
        const tracks = filterTracks(scope.tracks, normalized);
        const hasMatches = tracks.some(
          (track) => !track.synthetic && track.activities.length,
        );
        return hasMatches ? { ...scope, tracks } : null;
      })
      .filter(Boolean);
    return {
      ...layer,
      agentScopes,
      tracks: agentScopes.flatMap((scope) => scope.tracks),
    };
  });
}

export function flameOverviewLanes(model) {
  return (model?.layers ?? []).map((layer) => ({
    id: layer.id,
    tone: layer.id,
    intervals: layer.tracks
      .filter((track) => !track.synthetic)
      .flatMap((track) => track.activities.map((activity) => ({
        startOffsetMs: activity.startOffsetMs,
        durMs: activity.live
          ? activity.traceSpanMs - activity.startOffsetMs
          : activity.durMs,
        depth: activity.depth,
        status: activity.status,
        aggregate: activity.aggregate,
      }))),
  }));
}

export function flameActivityDetail(activity) {
  if (activity.density) {
    return {
      selectionId: activity.id,
      title: activity.label,
      kind: 'Viewport density summary',
      rows: {
        layer: activity.layer === 'agent' ? 'Agent' : 'Harness',
        lane: activity.toolEffect
          ? `${activity.toolEffect.toolLabel} effects`
          : GROUP_LABELS[activity.group] ?? activity.group,
        activities: activity.densityCount,
        interval: `+${formatOffset(activity.startOffsetMs)} · ${formatOffset(activity.durMs ?? 0)}`,
        detail: 'Zoom in to inspect the individual activities represented by this bar',
      },
      attributes: {},
      evidence: [],
      raw: null,
    };
  }
  const detail = actionDetail(activity.action);
  detail.title = activity.label;
  detail.rows = {
    layer: activity.layer === 'agent' ? 'Agent' : 'Harness',
    lane: activity.toolEffect
      ? `${activity.toolEffect.toolLabel} effects`
      : GROUP_LABELS[activity.group] ?? activity.group,
    classification: activity.layer === 'agent'
      ? 'Linked to the primary LLM/tool lineage'
      : 'Background-tagged or outside the primary lineage',
    start_offset: `+${formatOffset(activity.startOffsetMs)}`,
    ...detail.rows,
  };
  if (activity.agentScope) {
    detail.rows.agent_role = activity.agentScope.role === 'main'
      ? 'Main agent'
      : activity.agentScope.label;
    if (activity.agentScope.relationSource) {
      detail.rows.agent_relationship = activity.agentScope.relationSource;
    }
    if (activity.agentScope.parentLabel) {
      detail.rows.parent_agent = activity.agentScope.parentLabel;
    }
    if (activity.agentScope.spawnLabel) {
      detail.rows.spawned_by = activity.agentScope.spawnLabel;
    }
  }
  if (activity.backgroundKind) {
    detail.rows.background_kind = activity.backgroundKind;
    detail.rows.classification_source = activity.classificationSource;
  }
  if (activity.aggregate) {
    detail.rows.aggregate = `${activity.aggregateCount} activities from first to last observation`;
  }
  if (activity.summaryMarker) {
    detail.rows.display_interval = 'Observation summary rendered at its first event; raw timing remains available below';
  }
  if (activity.timingSource === 'derived_tool_execution') {
    detail.rows.display_interval = 'Derived execution window: assistant response end to matching tool-result request start';
  }
  if (activity.toolEffect) {
    detail.rows.classification = activity.toolEffect.preparation === 'shell_snapshot'
      ? `Observed shell preparation for Agent Tool ${activity.toolEffect.toolLabel}`
      : `Observed effect of Agent Tool ${activity.toolEffect.toolLabel}`;
    detail.rows.classification_source = 'Tool association + observed command process tree';
    detail.rows.agent_tool = activity.toolEffect.toolLabel;
    if (activity.toolEffect.preparation) {
      detail.rows.tool_phase = activity.toolEffect.preparation;
    }
    detail.rows.tool_process_root = activity.toolEffect.rootActionId;
    if (activity.toolEffect.toolCallId) {
      detail.rows.tool_call_action = activity.toolEffect.toolCallId;
    }
    if (activity.timingSource === 'observed_command_tree') {
      detail.rows.display_duration = formatOffset(activity.durMs ?? 0);
      detail.rows.display_interval = 'Observed command process tree envelope';
      detail.rows.raw_duration = detail.rows.duration;
      delete detail.rows.duration;
    }
  }
  return detail;
}

function filterTracks(tracks, normalized) {
  return tracks
    .map((track) => {
      const activities = track.activities
        .filter((activity) => activity.searchText.includes(normalized));
      if (track.synthetic) {
        return { ...track, activities };
      }
      return { ...track, ...packActivityDepths(activities) };
    })
    .filter((track) => track.activities.length > 0 || track.synthetic);
}

function buildAgentLayer(
  activities,
  actions,
  links,
  childrenByParent,
  window,
) {
  const linkedInvocationIds = new Set(
    (links ?? [])
      .filter((link) => link.valid !== false && link.role === 'agent.invocation.child_llm_request')
      .map((link) => link.parent),
  );
  const invocations = activities
    .filter((activity) => (
      activity.kind === 'agent.invocation'
      && linkedInvocationIds.has(activity.id)
    ))
    .sort(compareActivities);
  if (!invocations.length) {
    return buildLayer(
      'agent',
      'Agent layer',
      'Primary LLM turns and their linked tool chain',
      activities,
      window,
    );
  }

  const scopeModel = deriveAgentScopes(
    invocations,
    actions,
    links,
    childrenByParent,
  );
  const scopeById = new Map(scopeModel.map((scope) => [scope.id, scope]));
  const activityOwner = new Map();
  for (const scope of scopeModel) {
    for (const actionId of scope.actionIds) {
      activityOwner.set(actionId, scope.id);
    }
  }
  for (const activity of activities) {
    const toolCallId = activity.toolEffect?.toolCallId;
    const toolOwner = toolCallId ? activityOwner.get(toolCallId) : null;
    if (toolOwner && !activityOwner.has(activity.id)) {
      activityOwner.set(activity.id, toolOwner);
    }
  }

  const mainDefinition = {
    id: 'agent-main',
    role: 'main',
    label: 'Main agent',
    agentType: 'Primary',
    searchText: 'main agent primary',
    parentId: null,
    parentLabel: null,
    depth: 0,
    childCount: scopeModel.filter((scope) => scope.parentId === 'agent-main').length,
    invocation: null,
    spawnLabel: null,
    startOffsetMs: activityEnvelope(activities).startOffsetMs,
  };
  const definitions = [
    mainDefinition,
    ...scopeModel,
  ];
  const spawnedScopeByToolCall = new Map(
    scopeModel
      .filter((scope) => scope.parentToolCallId)
      .map((scope) => [scope.parentToolCallId, scope]),
  );
  const activitiesByScope = new Map(definitions.map((scope) => [scope.id, []]));

  for (const original of activities) {
    const ownerId = activityOwner.get(original.id) ?? mainDefinition.id;
    const scope = scopeById.get(ownerId) ?? mainDefinition;
    const spawnedScope = spawnedScopeByToolCall.get(original.id);
    const label = spawnedScope
      ? `Spawn subagent · ${spawnedScope.agentType}`
      : original.label;
    const agentScope = {
      id: scope.id,
      role: scope.role,
      label: scope.label,
      parentLabel: scope.parentLabel,
      spawnLabel: scope.spawnLabel,
      relationSource: scope.relationSource,
    };
    activitiesByScope.get(scope.id).push({
      ...original,
      label,
      agentScope,
      spawnedAgentScopeId: spawnedScope?.id ?? null,
      searchText: `${original.searchText} ${label} ${scope.searchText ?? scope.label}`
        .toLowerCase(),
    });
  }

  const orderedDefinitions = orderAgentScopes(definitions);
  const activityById = new Map(
    [...activitiesByScope.values()]
      .flat()
      .map((activity) => [activity.id, activity]),
  );
  const agentScopes = orderedDefinitions.map((definition, index) => buildAgentScope(
    definition,
    activitiesByScope.get(definition.id) ?? [],
    window,
    index,
    activityById.get(definition.parentToolCallId) ?? null,
  ));
  return {
    id: 'agent',
    label: 'Agent layer',
    description: `${1 + scopeModel.length} agent loops · primary and invoked subagents`,
    tracks: agentScopes.flatMap((scope) => scope.tracks),
    agentScopes,
    activityCount: activities.length,
    durationMs: coveredDuration(activities),
  };
}

function buildAgentScope(definition, activities, window, index, spawnActivity) {
  const { actionIds: _actionIds, ...scopeDefinition } = definition;
  const scopeId = definition.id;
  const trackActivities = activities.filter(
    (activity) => activity.kind !== 'agent.invocation',
  );
  const envelopeActivities = spawnActivity
    ? [spawnActivity]
    : (activities.length ? activities : trackActivities);
  const tracks = [];
  if (envelopeActivities.length) {
    tracks.push(syntheticTrack('agent', envelopeActivities, window, {
      id: `${scopeId}-scope`,
      trackLabel: definition.role === 'main'
        ? 'Agent loop'
        : 'Subagent loop',
      activityLabel: definition.role === 'main'
        ? 'Main agent activity envelope'
        : `${definition.label} activity envelope`,
    }));
  }
  for (const group of AGENT_GROUP_ORDER) {
    const grouped = trackActivities.filter((activity) => activity.group === group);
    if (!grouped.length) {
      continue;
    }
    if (group === 'detail') {
      tracks.push(...toolEffectTracks(grouped, scopeId));
      continue;
    }
    tracks.push({
      id: `${scopeId}-${group}`,
      label: GROUP_LABELS[group],
      description: GROUP_DESCRIPTIONS[group],
      group,
      synthetic: false,
      ...packActivityDepths(grouped),
    });
  }
  return {
    ...scopeDefinition,
    startOffsetMs: spawnActivity?.startOffsetMs ?? scopeDefinition.startOffsetMs,
    ordinal: definition.role === 'subagent' ? index : 0,
    tracks,
    activityCount: activities.length,
    durationMs: coveredDuration(envelopeActivities),
    llmCallCount: activities.filter((activity) => activity.kind === 'llm.call').length,
    toolCallCount: activities.filter((activity) => activity.kind === 'llm.tool_call').length,
    childCount: definition.childCount ?? 0,
  };
}

function buildLayer(id, label, description, activities, window) {
  const groups = id === 'agent' ? AGENT_GROUP_ORDER : HARNESS_GROUP_ORDER;
  const tracks = [];
  if (activities.length) {
    tracks.push(syntheticTrack(id, activities, window));
  }
  for (const group of groups) {
    const grouped = activities.filter((activity) => activity.group === group);
    if (grouped.length) {
      if (id === 'agent' && group === 'detail') {
        tracks.push(...toolEffectTracks(grouped));
        continue;
      }
      tracks.push({
        id: `${id}-${group}`,
        label: GROUP_LABELS[group],
        description: GROUP_DESCRIPTIONS[group],
        group,
        synthetic: false,
        ...packActivityDepths(grouped),
      });
    }
  }
  return {
    id,
    label,
    description,
    tracks,
    activityCount: activities.length,
    durationMs: coveredDuration(activities),
  };
}

function emptyLayer(id, label, description) {
  return { id, label, description, tracks: [], activityCount: 0, durationMs: 0 };
}

function syntheticTrack(layer, activities, window, options = {}) {
  const { startOffsetMs, endOffsetMs } = activityEnvelope(activities);
  const action = activities[0].action;
  return {
    id: options.id ?? `${layer}-scope`,
    label: options.trackLabel ?? (layer === 'agent' ? 'Agent loop' : 'Harness envelope'),
    group: 'scope',
    synthetic: true,
    depthCount: 1,
    activities: [{
      id: options.id ?? `${layer}-scope`,
      layer,
      group: 'scope',
      kind: `${layer}.scope`,
      label: options.activityLabel
        ?? (layer === 'agent' ? 'Agent activity envelope' : 'Harness activity envelope'),
      target: '',
      status: 'success',
      startOffsetMs,
      durMs: Math.max(endOffsetMs - startOffsetMs, 0.001),
      live: false,
      action,
      searchText: layer,
      synthetic: true,
      aggregate: true,
      aggregateCount: activities.length,
      traceSpanMs: window.spanMs,
    }],
  };
}

function activityFromAction(
  action,
  window,
  layer,
  callRequest,
  backgroundKind,
  callOrdinal,
  displayInterval,
  toolEffect,
) {
  const startNanos = displayInterval?.startNanos ?? parseNanos(action.start_time_unix_nanos);
  const endNanos = displayInterval?.endNanos
    ?? (action.end_time_unix_nanos ? parseNanos(action.end_time_unix_nanos) : null);
  const startOffsetMs = nanosDiffMs(startNanos, window.startNanos);
  const observedDurMs = endNanos === null ? null : Math.max(nanosDiffMs(endNanos, startNanos), 0);
  const summaryMarker = flameSummaryMarkerKind(action.kind);
  const durMs = summaryMarker ? 0 : observedDurMs;
  const target = semanticActionTarget(action) || '';
  const label = activityLabel(action, callRequest, backgroundKind, callOrdinal, toolEffect);
  const group = activityGroup(action.kind, layer, Boolean(toolEffect));
  const aggregateCount = actionAggregateCount(action);
  if (!group) {
    return null;
  }
  return {
    id: action.id,
    layer,
    group,
    kind: action.kind,
    label,
    target,
    status: action.status,
    startOffsetMs,
    durMs,
    live: endNanos === null,
    action,
    backgroundKind,
    timingSource: summaryMarker
      ? 'observation_summary_marker'
      : (displayInterval?.source ?? 'observed'),
    observedDurMs,
    summaryMarker,
    classificationSource: backgroundKind === 'auxiliary_inferred' ? 'inferred' : 'observed',
    toolEffect,
    aggregate: aggregateActionKind(action.kind),
    aggregateCount,
    synthetic: false,
    traceSpanMs: window.spanMs,
    searchText: [
      label,
      target,
      action.kind,
      action.status,
      backgroundKind,
      toolEffect?.toolLabel,
    ]
      .filter(Boolean)
      .join(' ')
      .toLowerCase(),
  };
}

function activityLabel(action, callRequest, backgroundKind, callOrdinal, toolEffect) {
  if (backgroundKind) {
    const label = BACKGROUND_LABELS[backgroundKind] ?? humanize(backgroundKind);
    if (action.kind === 'llm.request') {
      return `${label} request`;
    }
    if (action.kind === 'llm.response') {
      return `${label} response`;
    }
    return label;
  }
  if (action.kind === 'llm.call') {
    const model = semanticActionTarget(action)
      || callRequest?.attributes?.['llm.request.model'];
    const sequence = callOrdinal ? `LLM call ${callOrdinal}` : 'LLM call';
    return model ? `${sequence} · ${model}` : sequence;
  }
  if (action.kind === 'llm.request') {
    return 'User / context request';
  }
  if (action.kind === 'llm.response') {
    return 'Assistant response';
  }
  if (action.kind === 'llm.tool_call') {
    const name = action.attributes?.['llm.tool_call.name'];
    return name ? `tool.call:${name}` : 'tool.call';
  }
  if (action.kind === 'llm.tool_result') {
    return action.attributes?.['llm.tool_result.is_error'] === 'true'
      ? 'tool.result:error'
      : 'tool.result';
  }
  if (toolEffect?.root && action.kind === 'command.invocation') {
    const executable = basename(
      action.attributes?.['process.executable']
      ?? action.title,
    );
    if (toolEffect.preparation === 'shell_snapshot') {
      return executable
        ? `${toolEffect.toolLabel} environment setup · ${executable}`
        : `${toolEffect.toolLabel} environment setup`;
    }
    return executable
      ? `${toolEffect.toolLabel} execution · ${executable}`
      : `${toolEffect.toolLabel} execution`;
  }
  if (action.kind === 'process.fork_attempt') {
    const syscall = action.attributes?.syscall;
    return syscall ? `${syscall}()` : 'fork attempt';
  }
  if (action.kind === 'process.exec') {
    const syscall = action.attributes?.syscall ?? 'exec';
    const executable = basename(action.attributes?.['process.executable']);
    return executable ? `${syscall} · ${executable}` : syscall;
  }
  if (action.kind === 'process.exit') {
    const exitCode = action.attributes?.['process.exit_code'];
    return exitCode == null ? 'process exit' : `process exit · ${exitCode}`;
  }
  const aggregateCount = actionAggregateCount(action);
  if (aggregateActionKind(action.kind)) {
    return `${humanize(action.kind)} ×${aggregateCount}`;
  }
  return semanticActionLabel(action) || action.title || action.kind;
}

function actionAggregateCount(action) {
  const keys = {
    'file.read': 'file.read_count',
    'file.write': 'file.write_count',
    'file.tty_io': 'file.tty.event_count',
    'file.bulk_read': 'file.bulk_read.read_count',
    'fs.enumerate': 'fs.enumerate.unique_path_count',
  };
  const key = keys[action.kind];
  if (!key) {
    return 1;
  }
  const count = Number(action.attributes?.[key]);
  return Number.isFinite(count) && count > 1 ? count : 1;
}

function aggregateActionKind(kind) {
  return kind === 'file.read'
    || kind === 'file.write'
    || kind === 'file.tty_io'
    || kind === 'file.bulk_read'
    || kind === 'fs.enumerate';
}

function flameSummaryMarkerKind(kind) {
  return kind === 'file.tty_io';
}

function activityGroup(kind, layer, toolEffect = false) {
  if (kind === 'llm.call') {
    return 'model';
  }
  if (layer === 'agent') {
    if (kind === 'llm.request' || kind === 'llm.response') {
      return 'dialogue';
    }
    if (toolEffect) {
      return 'detail';
    }
    if (
      kind === 'llm.tool_call'
      || kind === 'llm.tool_result'
      || kind === 'agent.invocation'
      || kind === 'command.invocation'
      || kind.startsWith('mcp.')
    ) {
      return 'tools';
    }
    return kind.startsWith('file.') || kind === 'fs.enumerate' ? 'detail' : null;
  }
  if (kind === 'command.invocation' || kind === 'agent.invocation') {
    return 'commands';
  }
  if (kind.startsWith('file.') || kind === 'fs.enumerate') {
    return 'filesystem';
  }
  if (kind.startsWith('http.') || kind.startsWith('sse.') || kind.startsWith('mcp.')) {
    return 'protocol';
  }
  if (kind === 'llm.request' || kind === 'llm.response') {
    return 'dialogue';
  }
  if (kind === 'llm.tool_call' || kind === 'llm.tool_result') {
    return 'tools';
  }
  if (kind.startsWith('process.')) {
    return 'process';
  }
  if (
    kind === 'enforcement.decision'
  ) {
    return 'runtime';
  }
  return null;
}

function toolEffectTracks(activities, idPrefix = 'agent') {
  const groupedByToolCall = new Map();
  for (const activity of activities) {
    const key = activity.toolEffect?.toolCallId
      ?? activity.toolEffect?.rootActionId
      ?? 'unassigned';
    if (!groupedByToolCall.has(key)) {
      groupedByToolCall.set(key, []);
    }
    groupedByToolCall.get(key).push(activity);
  }
  return [...groupedByToolCall.entries()]
    .sort(([, left], [, right]) => compareActivities(left[0], right[0]))
    .map(([toolCallId, rows], index) => {
      const effect = rows[0]?.toolEffect;
      const toolLabel = effect?.toolLabel ?? 'Tool';
      return {
        id: `${idPrefix}-detail-${index + 1}-${toolCallId}`,
        label: effect ? `${toolLabel} effects` : GROUP_LABELS.detail,
        description: effect
          ? `Observed command, process, and file activity under the ${toolLabel} tool execution trees`
          : GROUP_DESCRIPTIONS.detail,
        group: 'detail',
        synthetic: false,
        ...packActivityDepths(rows),
      };
    });
}

function attributedToolEffects(
  attribution,
  actions,
  actionById,
  childrenByParent,
  displayIntervals,
  primaryAgentActionIds,
) {
  const effects = new Map();
  const primaryToolCalls = actions.filter(
    (action) => action.kind === 'llm.tool_call' && primaryAgentActionIds.has(action.id),
  );
  for (const segment of attribution?.segments ?? []) {
    if (segment.category !== 'agent_side') {
      continue;
    }
    const startNanos = parseNanos(segment.start_unix_nanos);
    const endNanos = parseNanos(segment.end_unix_nanos);
    if (endNanos < startNanos) {
      continue;
    }
    if (segment.subcategory !== 'tool') {
      continue;
    }
    const toolLabel = String(segment.label ?? segment.key ?? 'Tool');
    const toolCall = matchingToolCall(
      primaryToolCalls,
      displayIntervals,
      toolLabel,
      startNanos,
      endNanos,
    );
    for (const rootActionId of segment.action_ids ?? []) {
      const root = actionById.get(rootActionId);
      if (root?.kind !== 'command.invocation') {
        continue;
      }
      const attributionInfo = {
        toolLabel,
        toolCallId: toolCall?.id ?? null,
        rootActionId,
      };
      assignToolEffectTree(
        effects,
        rootActionId,
        attributionInfo,
        actionById,
        childrenByParent,
      );
    }
  }
  // Shared shell setup can be entirely hidden by concurrent model-side time.
  // Discover its process root independently of the exclusive accounting slices.
  for (const root of actions) {
    const commandLine = String(root.attributes?.['command.line'] ?? root.title ?? '');
    if (root.kind !== 'command.invocation'
      || effects.has(root.id)
      || !commandLine.includes('SNAPSHOT_FILE=')
      || !commandLine.includes('/shell-snapshots/')) {
      continue;
    }
    const toolCall = matchingShellSetupToolCall(
      root,
      primaryToolCalls,
      actionById,
      displayIntervals,
    );
    if (!toolCall) {
      continue;
    }
    assignToolEffectTree(effects, root.id, {
      toolLabel: 'Bash',
      toolCallId: toolCall.id,
      rootActionId: root.id,
      preparation: 'shell_snapshot',
    }, actionById, childrenByParent);
  }
  assignCommandMatchedToolEffects(
    effects,
    actions,
    primaryToolCalls,
    actionById,
    childrenByParent,
    displayIntervals,
  );
  return effects;
}

function matchingShellSetupToolCall(root, toolCalls, actionById, displayIntervals) {
  // A later tool call cannot own a process that was already initializing.
  const setupStartNanos = parseNanos(root.start_time_unix_nanos);
  const candidates = matchingToolCandidates(
    toolCalls,
    displayIntervals,
    'Bash',
    setupStartNanos,
    setupStartNanos,
  );
  if (!candidates.length) {
    return null;
  }
  const commandCandidates = candidates
    .map((toolCall) => ({
      toolCall,
      commandStartNanos: firstMatchingCommandStart(
        toolCall,
        actionById,
        displayIntervals,
        setupStartNanos,
      ),
    }))
    .filter(({ commandStartNanos }) => commandStartNanos != null)
    .sort((left, right) => (
      left.commandStartNanos < right.commandStartNanos
        ? -1
        : left.commandStartNanos > right.commandStartNanos ? 1 : 0
    ));
  if (commandCandidates.length) {
    const firstCommandStart = commandCandidates[0].commandStartNanos;
    const firstCommands = commandCandidates.filter(
      ({ commandStartNanos }) => commandStartNanos === firstCommandStart,
    );
    if (firstCommands.length === 1) {
      return firstCommands[0].toolCall;
    }
  }
  return candidates.length === 1 ? candidates[0] : null;
}

function firstMatchingCommandStart(
  toolCall,
  actionById,
  displayIntervals,
  setupStartNanos,
) {
  const command = toolCallCommand(toolCall, actionById);
  if (!command) {
    return null;
  }
  const interval = displayIntervals.get(toolCall.id) ?? observedInterval(toolCall);
  let firstStart = null;
  for (const action of actionById.values()) {
    if (action.kind !== 'command.invocation') {
      continue;
    }
    const commandLine = String(action.attributes?.['command.line'] ?? action.title ?? '');
    if (!commandMatches(commandLine, command)) {
      continue;
    }
    const start = parseNanos(action.start_time_unix_nanos);
    if (
      start < setupStartNanos
      || start < interval.startNanos
      || start > interval.endNanos
    ) {
      continue;
    }
    if (firstStart == null || start < firstStart) {
      firstStart = start;
    }
  }
  return firstStart;
}

function assignCommandMatchedToolEffects(
  effects,
  actions,
  toolCalls,
  actionById,
  childrenByParent,
  displayIntervals,
) {
  const inferred = new Map();
  const commandTools = toolCalls
    .map((action) => ({
      action,
      command: toolCallCommand(action, actionById),
      interval: displayIntervals.get(action.id) ?? observedInterval(action),
    }))
    .filter((candidate) => candidate.command);
  for (const root of actions) {
    const attributed = effects.get(root.id);
    if (root.kind !== 'command.invocation' || attributed?.toolCallId) {
      continue;
    }
    const commandLine = String(root.attributes?.['command.line'] ?? root.title ?? '');
    const rootStart = parseNanos(root.start_time_unix_nanos);
    const candidates = commandTools
      .filter((candidate) => !attributed
        || String(candidate.action.attributes?.['llm.tool_call.name'] ?? '').toLowerCase()
          === attributed.toolLabel.toLowerCase())
      .filter((candidate) => commandMatches(commandLine, candidate.command))
      .filter((candidate) => (
        candidate.interval.startNanos <= rootStart
        && rootStart <= candidate.interval.endNanos
      ));
    if (candidates.length !== 1) {
      continue;
    }
    const [tool] = candidates;
    assignToolEffectTree(
      inferred,
      root.id,
      {
        toolLabel: String(tool.action.attributes?.['llm.tool_call.name'] ?? 'Tool'),
        toolCallId: tool.action.id,
        rootActionId: root.id,
      },
      actionById,
      childrenByParent,
    );
  }
  for (const [actionId, effect] of inferred) {
    const attributed = effects.get(actionId);
    if (!attributed) {
      effects.set(actionId, effect);
    } else if (
      !attributed.toolCallId
      && attributed.rootActionId === effect.rootActionId
      && attributed.toolLabel.toLowerCase() === effect.toolLabel.toLowerCase()
    ) {
      // A category-level attribution confirms Bash work, not which invocation
      // owns it. Fill that missing identity without replacing observed timing.
      effects.set(actionId, { ...attributed, toolCallId: effect.toolCallId });
    }
  }
}

function commandMatches(commandLine, expected) {
  const observed = commandLine.trim();
  if (observed === expected) {
    return true;
  }
  // Recognize complete shell command arguments and the snapshot/eval wrapper.
  // Unknown wrappers stay unclaimed rather than matching text inside arguments.
  const wrapper = observed.match(/^(?:\/[^\s]+\/)?(?:ba|da|z|k)?sh\s+((?:-[a-z]+\s+)+)([\s\S]+)$/);
  if (!wrapper || !wrapper[1].split(/\s+/).some((flag) => flag.includes('c'))) {
    return false;
  }
  const argumentsForCommand = [
    expected,
    `'${expected.replaceAll("'", "'\\''")}'`,
    `"${expected.replace(/["\\$`]/g, '\\$&')}"`,
  ];
  const script = wrapper[2].trim();
  if (argumentsForCommand.includes(script)) {
    return true;
  }
  const execution = script
    .replace(/^source\s+(?:'[^']*'|"[^"]*"|[^\s;&|]+)(?:\s+2>\/dev\/null\s*\|\|\s*true)?\s*&&\s*/, '')
    .replace(/^setopt\s+NO_EXTENDED_GLOB\s+2>\/dev\/null\s*\|\|\s*true\s*&&\s*/, '')
    .replace(/\s*<\s*\/dev\/null(?:\s*&&\s*pwd\s+-P\s*>\|\s*(?:'[^']*'|"[^"]*"|[^\s;&|]+))?$/, '');
  return execution.startsWith('eval ')
    && argumentsForCommand.includes(execution.slice(5).trim());
}

function toolCallCommand(toolCall, actionById) {
  const args = toolCallArguments(toolCall, actionById);
  const command = args?.command ?? args?.cmd;
  return typeof command === 'string' && command.trim() ? command.trim() : null;
}

function assignToolEffectTree(
  effects,
  rootActionId,
  attributionInfo,
  actionById,
  childrenByParent,
) {
  // Accounting slices classify exclusive wall-clock time, not process ownership.
  // Once a root is associated, retain its whole tree and observed time envelope.
  const actionIds = new Set();
  collectDescendants(rootActionId, childrenByParent, actionIds);
  let { startNanos, endNanos } = observedInterval(actionById.get(rootActionId));
  for (const actionId of actionIds) {
    const action = actionById.get(actionId);
    if (!action) {
      continue;
    }
    const interval = observedInterval(action);
    // Summary lifetimes may end at trace finalization. They render as markers,
    // so only their event position contributes to the execution envelope.
    const observedEnd = flameSummaryMarkerKind(action.kind)
      ? interval.startNanos
      : interval.endNanos;
    if (interval.startNanos < startNanos) {
      startNanos = interval.startNanos;
    }
    if (observedEnd > endNanos) {
      endNanos = observedEnd;
    }
  }
  for (const actionId of actionIds) {
    if (actionById.has(actionId)) {
      effects.set(actionId, {
        ...attributionInfo,
        startNanos,
        endNanos,
        root: actionId === rootActionId,
      });
    }
  }
}

function matchingToolCall(toolCalls, displayIntervals, toolLabel, startNanos, endNanos) {
  const candidates = matchingToolCandidates(
    toolCalls,
    displayIntervals,
    toolLabel,
    startNanos,
    endNanos,
  );
  return candidates.length === 1 ? candidates[0] : null;
}

function matchingToolCandidates(toolCalls, displayIntervals, toolLabel, startNanos, endNanos) {
  const normalizedLabel = toolLabel.trim().toLowerCase();
  return toolCalls
    .filter((action) => {
      const name = String(action.attributes?.['llm.tool_call.name'] ?? '').trim().toLowerCase();
      return !normalizedLabel || name === normalizedLabel;
    })
    .map((action) => {
      const interval = displayIntervals.get(action.id) ?? observedInterval(action);
      return { action, overlap: intervalOverlapNanos(interval, startNanos, endNanos) };
    })
    .filter((candidate) => candidate.overlap >= 0n)
    .sort((left, right) => left.overlap > right.overlap ? -1 : left.overlap < right.overlap ? 1 : 0)
    .map((candidate) => candidate.action);
}

function attributedDisplayInterval(displayInterval, toolEffect) {
  if (toolEffect?.root) {
    return {
      startNanos: toolEffect.startNanos,
      endNanos: toolEffect.endNanos,
      source: 'observed_command_tree',
    };
  }
  return displayInterval;
}

function observedInterval(action) {
  const startNanos = parseNanos(action.start_time_unix_nanos);
  return {
    startNanos,
    endNanos: action.end_time_unix_nanos
      ? parseNanos(action.end_time_unix_nanos)
      : startNanos,
  };
}

function intervalOverlapNanos(interval, startNanos, endNanos) {
  const start = interval.startNanos > startNanos ? interval.startNanos : startNanos;
  const end = interval.endNanos < endNanos ? interval.endNanos : endNanos;
  return end >= start ? end - start : -1n;
}

function derivedDisplayIntervals(actions, links, actionById) {
  const resultByToolCall = new Map();
  for (const link of links ?? []) {
    if (
      link.valid
      && link.role === 'llm.tool_call.result'
      && actionById.get(link.parent)?.kind === 'llm.tool_call'
      && actionById.get(link.child)?.kind === 'llm.tool_result'
    ) {
      resultByToolCall.set(link.parent, actionById.get(link.child));
    }
  }

  const intervals = new Map();
  for (const action of actions) {
    if (action.kind !== 'llm.tool_call') {
      continue;
    }
    const responseId = action.attributes?.['llm.tool_call.response_action_id'];
    const response = responseId ? actionById.get(responseId) : null;
    const result = resultByToolCall.get(action.id);
    if (!response || !result) {
      continue;
    }
    const startNanos = response.end_time_unix_nanos
      ? parseNanos(response.end_time_unix_nanos)
      : parseNanos(response.start_time_unix_nanos);
    const endNanos = parseNanos(result.start_time_unix_nanos);
    if (endNanos < startNanos) {
      continue;
    }
    intervals.set(action.id, {
      startNanos,
      endNanos,
      source: 'derived_tool_execution',
    });
  }
  return intervals;
}

function graphChildren(links, actionById) {
  const children = new Map();
  for (const link of links ?? []) {
    if (!link.valid || !AGENT_GRAPH_ROLES.has(link.role)) {
      continue;
    }
    if (!actionById.has(link.parent) || !actionById.has(link.child)) {
      continue;
    }
    if (!children.has(link.parent)) {
      children.set(link.parent, []);
    }
    children.get(link.parent).push(link.child);
  }
  return children;
}

function resolveCallRequest(call, actionById, childrenByParent) {
  const requestId = call.attributes?.['llm.call.request_action_id'];
  const direct = requestId ? actionById.get(requestId) : null;
  if (direct?.kind === 'llm.request') {
    return direct;
  }
  return (childrenByParent.get(call.id) ?? [])
    .map((id) => actionById.get(id))
    .find((action) => action?.kind === 'llm.request') ?? null;
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

function packActivityDepths(activities) {
  const depthEnds = [];
  const packed = [];
  const sorted = [...activities].sort(compareActivities);
  for (const activity of sorted) {
    const start = activity.startOffsetMs;
    let depth = depthEnds.findIndex((endMs) => endMs <= start);
    if (depth < 0) {
      depth = depthEnds.length;
    }
    depthEnds[depth] = activityEndMs(activity);
    packed.push({ ...activity, depth });
  }
  return {
    activities: packed,
    depthCount: Math.max(depthEnds.length, 1),
  };
}

function inferAuxiliaryBackgroundCalls(actions, requestByCall, backgroundKindByCall, links, requestContexts) {
  const childRequestIds = new Set(links
    .filter((link) => link.valid !== false && link.role === 'agent.invocation.child_llm_request')
    .map((link) => link.child));
  const calls = actions.filter((action) => action.kind === 'llm.call');
  for (const call of calls) {
    if (backgroundKindByCall.has(call.id)) {
      continue;
    }
    const request = requestByCall.get(call.id);
    if (!request || requestContexts.has(request.id) || childRequestIds.has(request.id) || !smallStandaloneRequest(request)) {
      continue;
    }
    const candidateBytes = numericAttribute(request, 'llm.request.canonical_body_bytes');
    const overlapsPrimary = calls.some((other) => {
      if (other.id === call.id || backgroundKindByCall.has(other.id)) {
        return false;
      }
      const otherRequest = requestByCall.get(other.id);
      const otherBytes = numericAttribute(otherRequest, 'llm.request.canonical_body_bytes');
      const candidatePreview = requestPreview(request);
      const sameUserPreview = Boolean(candidatePreview)
        && requestPreview(otherRequest) === candidatePreview;
      const nearConcurrentSidecar = candidateBytes <= 4_096
        && callStartDeltaMs(call, other) <= 100;
      return otherRequest
        && intervalsOverlap(call, other)
        && otherBytes >= Math.max(candidateBytes * 4, 32_768)
        && (sameUserPreview || nearConcurrentSidecar);
    });
    if (overlapsPrimary) {
      backgroundKindByCall.set(call.id, 'auxiliary_inferred');
    }
  }
}

function backgroundKindsByAction(backgroundKindByCall, childrenByParent, actionById) {
  const kinds = new Map();
  for (const [callId, backgroundKind] of backgroundKindByCall) {
    const descendants = new Set();
    collectDescendants(callId, childrenByParent, descendants);
    for (const actionId of descendants) {
      if (BACKGROUND_LLM_KINDS.has(actionById.get(actionId)?.kind)) {
        kinds.set(actionId, backgroundKind);
      }
    }
  }
  return kinds;
}

function smallStandaloneRequest(request) {
  const trajectory = request.attributes?.['llm.request.trajectory_id'];
  return trajectory === request.id
    && numericAttribute(request, 'llm.request.block_count') <= 2
    && numericAttribute(request, 'llm.request.user_message_count') <= 1
    && numericAttribute(request, 'llm.request.canonical_body_bytes') <= 16_384;
}

function requestPreview(request) {
  return String(request?.attributes?.['llm.request.message_preview'] ?? '').trim();
}

function numericAttribute(action, key) {
  const value = Number(action?.attributes?.[key]);
  return Number.isFinite(value) ? value : Number.POSITIVE_INFINITY;
}

function intervalsOverlap(left, right) {
  const leftStart = parseNanos(left.start_time_unix_nanos);
  const rightStart = parseNanos(right.start_time_unix_nanos);
  const leftEnd = left.end_time_unix_nanos ? parseNanos(left.end_time_unix_nanos) : leftStart;
  const rightEnd = right.end_time_unix_nanos ? parseNanos(right.end_time_unix_nanos) : rightStart;
  return leftStart < rightEnd && rightStart < leftEnd;
}

function callStartDeltaMs(left, right) {
  return Number(absBigInt(
    parseNanos(left.start_time_unix_nanos) - parseNanos(right.start_time_unix_nanos),
  )) / 1_000_000;
}

function absBigInt(value) {
  return value < 0n ? -value : value;
}

function computeWindow(actions) {
  let startNanos = null;
  let endNanos = null;
  let startIso = null;
  let endIso = null;
  for (const action of actions) {
    const start = parseNanos(action.start_time_unix_nanos);
    const end = action.end_time_unix_nanos ? parseNanos(action.end_time_unix_nanos) : start;
    if (startNanos === null || start < startNanos) {
      startNanos = start;
      startIso = action.start_time;
    }
    if (endNanos === null || end > endNanos) {
      endNanos = end;
      endIso = action.end_time ?? action.start_time;
    }
  }
  return {
    startNanos,
    endNanos,
    spanMs: Math.max(nanosDiffMs(endNanos, startNanos), 1),
    startIso,
    endIso,
  };
}

function coveredDuration(activities) {
  if (!activities.length) {
    return 0;
  }
  const intervals = activities
    .map((activity) => [activity.startOffsetMs, activityEndMs(activity)])
    .sort((left, right) => left[0] - right[0]);
  let total = 0;
  let [start, end] = intervals[0];
  for (const [nextStart, nextEnd] of intervals.slice(1)) {
    if (nextStart <= end) {
      end = Math.max(end, nextEnd);
    } else {
      total += end - start;
      start = nextStart;
      end = nextEnd;
    }
  }
  return total + end - start;
}

function activityEnvelope(activities) {
  let startOffsetMs = activities.length ? Number.POSITIVE_INFINITY : 0;
  let endOffsetMs = activities.length ? Number.NEGATIVE_INFINITY : 0;
  for (const activity of activities) {
    startOffsetMs = Math.min(startOffsetMs, activity.startOffsetMs);
    endOffsetMs = Math.max(endOffsetMs, activityEndMs(activity));
  }
  return { startOffsetMs, endOffsetMs };
}

function activityEndMs(activity) {
  return activity.live
    ? activity.traceSpanMs
    : activity.startOffsetMs + Math.max(activity.durMs ?? 0, 0.001);
}

function validTimedAction(action) {
  return Boolean(action?.id && action?.kind && action.start_time_unix_nanos != null);
}

function compareActivities(left, right) {
  return left.startOffsetMs - right.startOffsetMs || String(left.id).localeCompare(String(right.id));
}

function compareTimedActions(left, right) {
  const byStart = parseNanos(left.start_time_unix_nanos) - parseNanos(right.start_time_unix_nanos);
  if (byStart < 0n) {
    return -1;
  }
  if (byStart > 0n) {
    return 1;
  }
  return String(left.id).localeCompare(String(right.id));
}

function parseNanos(value) {
  try {
    return BigInt(value ?? 0);
  } catch {
    return 0n;
  }
}

function nanosDiffMs(later, earlier) {
  return Number(later - earlier) / 1_000_000;
}

function humanize(value) {
  return String(value)
    .replace(/[._-]+/g, ' ')
    .replace(/^./, (letter) => letter.toUpperCase());
}

function basename(value) {
  const normalized = String(value ?? '').replaceAll('\\', '/');
  return normalized.split('/').filter(Boolean).at(-1) ?? '';
}
