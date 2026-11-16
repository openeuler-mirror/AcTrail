import { computed, onBeforeUnmount, provide, watch } from 'vue';

import { markersByCall } from './model';
import { useCheckpointHistory } from './useCheckpointHistory';

export const CHECKPOINT_MARKERS = Symbol('checkpoint-markers');

/**
 * Lets ActionTreeNode show where saves and restores happened. `refreshNonce` changes when the header
 * asks for a refresh; `open` receives the clicked marker.
 */
export function provideCheckpointMarkers(traceKey, refreshNonce, open) {
  const history = useCheckpointHistory(traceKey);
  watch(refreshNonce, history.refresh);
  const byCall = computed(() => markersByCall(history.listing.value));
  provide(CHECKPOINT_MARKERS, {
    forAction: (actionId) => byCall.value.get(actionId) ?? [],
    open,
  });
  onBeforeUnmount(history.stop);
}
