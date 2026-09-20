import { inferAgentRequestLinks } from '../agent-scopes.js';
import { ToolCallDisplay } from '../../../shared/toolCallDisplay.js';
import { FLAME_GRAPH_ASSOCIATION_ROLES, STRUCTURAL_KINDS, BACKGROUND_LLM_KINDS, graphChildren, resolveCallRequest, collectDescendants, packActivityDepths, computeWindow, validTimedAction, compareTimedActions, parseNanos } from './utils.js';
import { buildAgentLayer, buildLayer, emptyLayer, activityFromAction } from './presentation.js';
import { attributedToolEffects, attributedDisplayInterval, derivedDisplayIntervals } from './tool-effects.js';

export function buildFlameGraph(actions, links, attribution = null, associations = [], requestContexts = new Map()) {
  const validActions = (actions ?? []).filter(validTimedAction);
  if (!validActions.length) {
    return emptyFlameGraphModel();
  }

  const graphLinks = flameGraphLinks(links, associations);
  const window = computeWindow(validActions);
  const actionById = new Map(validActions.map((action) => [action.id, action]));
  const toolDisplay = new ToolCallDisplay(actionById);
  const callMessages = new Set(graphLinks
    .filter((link) => link.valid !== false
      && ['llm.call.request', 'llm.call.response'].includes(link.role)
      && actionById.get(link.parent)?.kind === 'llm.call')
    .map((link) => link.child));
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
      toolDisplay,
    );
    if (!activity) {
      continue;
    }
    activity.modelMessage = callMessages.has(action.id)
      && ['llm.request', 'llm.response'].includes(action.kind);
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

export function filterFlameGraph(model, query, { showModelMessages = false } = {}) {
  const normalized = String(query ?? '').trim().toLowerCase();
  if (!normalized && showModelMessages) {
    return model.layers;
  }
  return model.layers.map((layer) => {
    if (!layer.agentScopes?.length) {
      return { ...layer, tracks: filterTracks(layer.tracks, normalized, showModelMessages) };
    }
    const agentScopes = layer.agentScopes
      .map((scope) => {
        const scopeMatches = (scope.searchText ?? '').includes(normalized);
        const tracks = filterTracks(scope.tracks, scopeMatches ? '' : normalized, showModelMessages);
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

function filterTracks(tracks, normalized, showModelMessages) {
  return tracks
    .map((track) => {
      const activities = track.activities
        .filter((activity) => (showModelMessages || !activity.modelMessage)
          && activity.searchText.includes(normalized));
      if (track.synthetic) {
        return { ...track, activities };
      }
      return { ...track, ...packActivityDepths(activities) };
    })
    .filter((track) => track.activities.length > 0 || track.synthetic);
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
