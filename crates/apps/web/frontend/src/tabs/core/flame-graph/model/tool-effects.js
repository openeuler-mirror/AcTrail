import { parseNanos } from './utils.js';

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
