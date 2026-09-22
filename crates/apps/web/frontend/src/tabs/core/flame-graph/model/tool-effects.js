import { toolCallArguments } from '../request-context.js';
import { flameSummaryMarkerKind, observedInterval, intervalOverlapNanos, collectDescendants, parseNanos } from './utils.js';

export function attributedToolEffects(
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

export function attributedDisplayInterval(displayInterval, toolEffect) {
  if (toolEffect?.root) {
    return {
      startNanos: toolEffect.startNanos,
      endNanos: toolEffect.endNanos,
      source: 'observed_command_tree',
    };
  }
  return displayInterval;
}

export function derivedDisplayIntervals(actions, links, actionById) {
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
