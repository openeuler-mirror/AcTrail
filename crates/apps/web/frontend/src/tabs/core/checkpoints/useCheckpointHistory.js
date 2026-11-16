import { ref, watch } from 'vue';

import { readTraceCheckpoints } from '../../../api';
import { needsPolling } from './model';

/** Reads the checkpoint listing of the trace and keeps it fresh while work is in progress. */
export function useCheckpointHistory(traceKey, { pollWhile = needsPolling, intervalMs = 1500 } = {}) {
  const listing = ref(null);
  const error = ref('');
  const loading = ref(false);
  let controller = null;
  let timer = null;
  let shownTrace = null;
  let stopped = false;
  let pollIntervalMs = intervalMs;

  async function refresh() {
    if (stopped) return;
    const traceId = traceKey();
    clearTimeout(timer);
    controller?.abort();
    controller = null;
    if (traceId !== shownTrace) {
      listing.value = null;
      error.value = '';
      shownTrace = traceId;
    }
    if (traceId == null) {
      listing.value = null;
      loading.value = false;
      return;
    }
    controller = new AbortController();
    const own = controller;
    loading.value = true;
    try {
      const data = await readTraceCheckpoints(traceId, { signal: own.signal });
      if (own !== controller) return;
      listing.value = data;
      pollIntervalMs = data.poll_interval_ms ?? intervalMs;
      error.value = '';
      if (pollWhile(data)) {
        timer = setTimeout(refresh, pollIntervalMs);
      }
    } catch (err) {
      if (err.name !== 'AbortError' && own === controller) {
        error.value = String(err.message ?? err);
        timer = setTimeout(refresh, pollIntervalMs);
      }
    } finally {
      if (own === controller) loading.value = false;
    }
  }

  function stop() {
    stopped = true;
    clearTimeout(timer);
    controller?.abort();
    controller = null;
  }

  watch(traceKey, refresh, { immediate: true });

  return { listing, error, loading, refresh, stop };
}
