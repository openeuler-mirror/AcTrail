import { reactive, ref, shallowRef } from 'vue';
import { readLlmRequestsActivity } from '../../api';

export class DashboardActivity {
  constructor() {
    const now = Date.now();
    this.draft = reactive({ from: this.localDate(now - 7 * 86_400_000), to: this.localDate(now) });
    this.data = shallowRef(null);
    this.loading = ref(false);
    this.error = ref('');
    this.query = null;
    this.controller = null;
  }

  localDate(milliseconds) {
    const date = new Date(milliseconds);
    return new Date(milliseconds - date.getTimezoneOffset() * 60_000).toISOString().slice(0, 19);
  }

  cancel() {
    this.controller?.abort();
    this.controller = null;
    this.loading.value = false;
  }

  submit() {
    const fromMs = new Date(this.draft.from).getTime();
    const toMs = new Date(this.draft.to).getTime();
    if (!Number.isFinite(fromMs) || !Number.isFinite(toMs) || fromMs >= toMs) return;
    return this.load({ fromMs, toMs });
  }

  refresh() { if (this.query) return this.load(this.query); }

  async load(query) {
    this.cancel();
    this.query = query;
    const controller = new AbortController();
    this.controller = controller;
    this.loading.value = true;
    this.error.value = '';
    try {
      const result = await readLlmRequestsActivity({ ...query, signal: controller.signal });
      if (this.controller === controller) this.data.value = result;
    } catch (error) {
      if (this.controller === controller && error.name !== 'AbortError') this.error.value = String(error.message ?? error);
    } finally {
      if (this.controller === controller) { this.controller = null; this.loading.value = false; }
    }
  }
}
