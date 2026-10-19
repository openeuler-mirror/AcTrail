import { ref, shallowRef } from 'vue';
import { readIdleIntervals } from '../../../../api';

export class IdlePages {
  constructor() {
    this.data = shallowRef(this.empty());
    this.loading = ref(false);
    this.error = ref('');
    this.limit = ref(100);
    this.range = shallowRef({});
    this.traceId = null;
    this.controller = null;
    this.retry = null;
  }

  empty() { return { intervals: [], axisEnd: null, page: 0, hasMore: false, cursor: null, scanned: 0 }; }

  cancel() {
    this.controller?.abort();
    this.controller = null;
    this.loading.value = false;
  }

  reset(traceId, actions = [], { from = '', to = '' } = {}) {
    this.cancel();
    this.traceId = traceId;
    this.data.value = this.empty();
    this.error.value = '';
    this.retry = null;
    if (!from && !to && actions.length) {
      let earliest = null;
      let latest = null;
      let live = false;
      for (const action of actions) {
        if (!action.start_time_unix_nanos) continue;
        const start = BigInt(action.start_time_unix_nanos);
        const end = BigInt(action.end_time_unix_nanos ?? action.start_time_unix_nanos);
        if (earliest === null || start < earliest) earliest = start;
        if (latest === null || end > latest) latest = end;
        live ||= action.end_time_unix_nanos == null;
      }
      from = earliest?.toString() ?? '';
      to = live ? '' : latest?.toString() ?? '';
    }
    this.range.value = { from, to };
  }

  async load({ cursor = null, page = 1, limit = this.limit.value } = {}) {
    if (this.traceId == null) return;
    this.cancel();
    const controller = new AbortController();
    this.controller = controller;
    this.loading.value = true;
    this.error.value = '';
    this.retry = () => this.load({ cursor, page, limit });
    try {
      const result = await readIdleIntervals(this.traceId, { ...this.range.value, cursor, limit, signal: controller.signal });
      if (this.controller !== controller) return;
      this.loadedLimit = limit;
      this.data.value = { intervals: result.idle_intervals ?? [], axisEnd: result.axis_end_unix_nanos,
        cursor: result.next_cursor, hasMore: Boolean(result.has_more), scanned: result.scanned ?? 0, page };
    } catch (error) {
      if (this.controller === controller && error.name !== 'AbortError') this.error.value = String(error.message ?? error);
    } finally {
      if (this.controller === controller) { this.controller = null; this.loading.value = false; }
    }
  }

  next() {
    const current = this.data.value;
    if (!this.loading.value && current.hasMore && current.cursor) {
      return this.load({ cursor: current.cursor, page: current.page + 1, limit: this.loadedLimit });
    }
  }
}
