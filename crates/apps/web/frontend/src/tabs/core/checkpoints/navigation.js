import { readCheckpointNavigation } from '../../../api';

/**
 * The ActionTree path to the LLM call an operation matched, in the shape the ActionTree's own
 * LLM navigation uses, or null when the observation data has no match.
 */
export async function llmCallPath(traceId, operationId) {
  const data = await readCheckpointNavigation(traceId, operationId);
  return data?.found && Array.isArray(data.path) && data.path.length ? data.path : null;
}
