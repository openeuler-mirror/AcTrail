import { parseNanos } from './utils.js';

// A result answers the call with the same tool call id in the same process.
// Ids can repeat within a trace, so the first result after the call answers it.
function resultsByToolCallId(actions) {
  const results = new Map();
  for (const action of actions) {
    const id = action.kind === 'llm.tool_result' ? action.attributes?.['llm.tool_call.id'] : null;
    if (!id) {
      continue;
    }
    const key = `${action.process?.process_id}\u0000${id}`;
    if (!results.has(key)) {
      results.set(key, []);
    }
    results.get(key).push(action);
  }
  return results;
}

function answeringResult(toolCall, results) {
  const id = toolCall.attributes?.['llm.tool_call.id'];
  const callStart = parseNanos(toolCall.start_time_unix_nanos);
  let answer = null;
  for (const result of (id ? results.get(`${toolCall.process?.process_id}\u0000${id}`) : null) ?? []) {
    const resultStart = parseNanos(result.start_time_unix_nanos);
    if (resultStart >= callStart
      && (!answer || resultStart < parseNanos(answer.start_time_unix_nanos))) {
      answer = result;
    }
  }
  return answer;
}

export function derivedDisplayIntervals(actions, actionById) {
  const results = resultsByToolCallId(actionById.values());

  const intervals = new Map();
  for (const action of actions) {
    if (action.kind !== 'llm.tool_call') {
      continue;
    }
    const responseId = action.attributes?.['llm.tool_call.response_action_id'];
    const response = responseId ? actionById.get(responseId) : null;
    const result = answeringResult(action, results);
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
