import { semanticActionLabel, semanticActionTarget } from '../../../actionLabels.js';
import { ToolCallDisplay } from '../../../shared/toolCallDisplay.js';
import { actionDetail, formatOffset } from '../../waterfall/model.js';
import { deriveAgentScopes, orderAgentScopes } from '../agent-scopes.js';
import { BACKGROUND_LABELS, AGENT_GROUP_ORDER, HARNESS_GROUP_ORDER, GROUP_LABELS, GROUP_DESCRIPTIONS, flameSummaryMarkerKind, packActivityDepths, coveredDuration, activityEnvelope, compareActivities, parseNanos, nanosDiffMs, humanize, basename } from './utils.js';

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
  const detail = actionDetail(activity.action, null, activity.target);
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

export function buildAgentLayer(
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

export function buildLayer(id, label, description, activities, window) {
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

export function emptyLayer(id, label, description) {
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

export function activityFromAction(
  action,
  window,
  layer,
  callRequest,
  backgroundKind,
  callOrdinal,
  displayInterval,
  toolEffect,
  toolDisplay,
) {
  const startNanos = displayInterval?.startNanos ?? parseNanos(action.start_time_unix_nanos);
  const endNanos = displayInterval?.endNanos
    ?? (action.end_time_unix_nanos ? parseNanos(action.end_time_unix_nanos) : null);
  const startOffsetMs = nanosDiffMs(startNanos, window.startNanos);
  const observedDurMs = endNanos === null ? null : Math.max(nanosDiffMs(endNanos, startNanos), 0);
  const summaryMarker = flameSummaryMarkerKind(action.kind);
  const durMs = summaryMarker ? 0 : observedDurMs;
  const target = toolDisplay.target(action);
  const label = action.kind === 'llm.tool_call'
    ? toolDisplay.label(action)
    : activityLabel(action, callRequest, backgroundKind, callOrdinal, toolEffect);
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
    statusLabel: ToolCallDisplay.statusLabel(action),
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
      ToolCallDisplay.statusLabel(action),
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
  if (action.kind === 'command.invocation' && !toolEffect?.root) {
    return semanticActionTarget(action) || 'Command';
  }
  if (toolEffect?.root && action.kind === 'command.invocation') {
    const executable = semanticActionTarget(action)
      || action.attributes?.['process.executable'];
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
