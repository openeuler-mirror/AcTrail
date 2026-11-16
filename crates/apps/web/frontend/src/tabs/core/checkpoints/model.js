/** Turns the checkpoint listing into the history rows of the tab and the markers of the ActionTree. */

export function formatTime(millis) {
  return millis == null ? '' : new Date(Number(millis)).toLocaleString();
}

export function formatClock(millis) {
  return millis == null ? '' : new Date(Number(millis)).toLocaleTimeString();
}

/** The status of an operation, as one of: waiting, running, succeeded, failed, unknown. */
export function operationStatus(operation) {
  if (operation.phase === 'waiting_boundary') return 'waiting';
  if (operation.phase !== 'finished') return 'running';
  return operation.outcome ?? 'unknown';
}

/** The status of a checkpoint, as one of: restorable, invalidated, deleted. */
export function checkpointStatus(checkpoint) {
  if (checkpoint.deleted_at != null) return 'deleted';
  if (checkpoint.invalidated_by != null) return 'invalidated';
  return 'restorable';
}

/** The restore of this trace that has not finished yet, if any. */
export function pendingRestore(listing) {
  return (listing?.operations ?? []).find(
    (operation) => operation.kind === 'restore' && operation.phase !== 'finished',
  ) ?? null;
}

export function restorableCount(listing) {
  return (listing?.checkpoints ?? []).filter((checkpoint) => checkpointStatus(checkpoint) === 'restorable').length;
}

/** Refresh live Traces, held processes or responses, and work that outlives command acceptance. */
export function needsPolling(listing) {
  return Boolean(listing?.trace && listing.trace.ended_at == null)
    || Boolean(listing?.runtime?.held || listing?.runtime?.busy)
    || (listing?.operations ?? []).some((operation) => operation.phase !== 'finished')
    || (listing?.checkpoints ?? []).some((checkpoint) => checkpoint.release === 'pending');
}

/**
 * A failed snapshot release is retried with the same delete command. Once the Trace is
 * disconnected, the release belongs to its disconnect, which only a live backend can retry.
 */
export function canDelete(checkpoint, pending, trace) {
  if (trace?.disconnected_at != null) return trace.disconnect_retryable && checkpoint.release === 'failed';
  return pending?.target_checkpoint !== checkpoint.checkpoint_id
    && (checkpoint.deleted_at == null || checkpoint.release === 'failed');
}

/**
 * Operations at LLM boundaries keyed by the ActionTree LLM call they matched, for the markers
 * on those calls.
 */
export function markersByCall(listing) {
  const markers = new Map();
  const checkpoints = new Map((listing?.checkpoints ?? []).map((checkpoint) => [checkpoint.checkpoint_id, checkpoint]));
  for (const operation of listing?.operations ?? []) {
    const callId = operation.llm_call_action_id;
    if (!callId) continue;
    const list = markers.get(callId) ?? [];
    const checkpointId = operation.kind === 'save' ? operation.created_checkpoint : operation.target_checkpoint;
    const checkpoint = checkpoints.get(checkpointId);
    list.push({
      operationId: operation.operation_id,
      kind: operation.kind,
      phase: operation.phase,
      outcome: operation.outcome,
      checkpointId,
      unavailable: checkpoint != null && checkpointStatus(checkpoint) !== 'restorable',
    });
    markers.set(callId, list);
  }
  return markers;
}
