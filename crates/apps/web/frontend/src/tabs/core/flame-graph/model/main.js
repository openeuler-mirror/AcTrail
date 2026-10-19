// Project bounded action windows onto the shared trace origin.
import { ToolCallDisplay } from '../../../shared/toolCallDisplay.js';
import { FLAME_GRAPH_ASSOCIATION_ROLES, BACKGROUND_LLM_KINDS, graphChildren, resolveCallRequest, collectDescendants, packActivityDepths, computeWindow, validTimedAction, compareTimedActions } from './utils.js';
import { buildAgentLayer, buildLayer, emptyLayer, activityFromAction } from './presentation.js';
import { derivedDisplayIntervals } from './tool-effects.js';

export function buildFlameGraph(actions, links, associations = [], t, contextActions = [], timelineWindow = null) {
  const validActions = (actions ?? []).filter(validTimedAction);
  if (!validActions.length) {
    return { ...emptyFlameGraphModel(t), ...(timelineWindow ? { window: timelineWindow } : {}) };
  }

  const graphLinks = flameGraphLinks(links, associations);
  const window = timelineWindow ?? computeWindow(validActions);
  const actionById = new Map([...contextActions, ...validActions].map((action) => [action.id, action]));
  const toolDisplay = new ToolCallDisplay(actionById);
  const displayedIds = new Set(validActions.map((action) => action.id));
  const callMessages = new Set(graphLinks
    .filter((link) => link.valid !== false
      && ['llm.call.request', 'llm.call.response'].includes(link.role)
      && displayedIds.has(link.parent) && actionById.get(link.parent)?.kind === 'llm.call')
    .map((link) => link.child));
  const displayIntervals = derivedDisplayIntervals(validActions, graphLinks, actionById);
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
    const backgroundKind = request?.attributes?.['llm.request.background_kind'];
    if (backgroundKind && !childRequests.has(request?.id)) {
      backgroundKindByCall.set(action.id, backgroundKind);
    }
  }
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

  const agentActivities = [];
  const harnessActivities = [];
  for (const action of validActions) {
    const backgroundKind = backgroundKindByAction.get(action.id) ?? null;
    const layer = backgroundKind ? 'harness' : (agentActionIds.has(action.id) ? 'agent' : 'harness');
    const activity = activityFromAction(
      action,
      window,
      layer,
      requestByCall.get(action.id) ?? null,
      backgroundKind,
      callOrdinalById.get(action.id) ?? null,
      displayIntervals.get(action.id) ?? null,
      null,
      toolDisplay,
      t,
    );
    if (!activity) {
      continue;
    }
    activity.scopeIncomplete = true;
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
      t,
    ),
    buildLayer('harness', t('partialOther'), t('partialOwnership'), harnessActivities, window, t),
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

export function emptyFlameGraphModel(t) {
  return {
    window: {
      startNanos: 0n,
      endNanos: 0n,
      spanMs: 1,
      startIso: null,
      endIso: null,
    },
    layers: [
      emptyLayer('agent', t('layer.agent'), t('layer.agentDescription')),
      emptyLayer('harness', t('layer.harness'), t('layer.harnessDescription')),
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

