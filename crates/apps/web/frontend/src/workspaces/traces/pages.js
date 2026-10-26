// Keep only the selected trace-list page and discard superseded responses.
import { ref, shallowRef } from 'vue';

export class TracePages {
  constructor(readPage) {
    this.readPage = readPage;
    this.rows = shallowRef([]);
    this.state = shallowRef({ page: 0, total: 0, totalPages: 0 });
    this.filters = shallowRef({ name: '', trace_id: '', from: '', to: '', limit: 50 });
    this.loading = ref(false);
    this.error = ref('');
    this.controller = null;
    this.retry = null;
  }

  cancel() {
    this.controller?.abort();
    this.controller = null;
    this.loading.value = false;
  }

  async load({ filters = this.filters.value, page = 1 } = {}) {
    this.cancel();
    const controller = new AbortController();
    this.controller = controller;
    this.loading.value = true;
    this.error.value = '';
    const query = { ...filters };
    this.retry = () => this.load({ filters: query, page });
    try {
      const result = await this.readPage({ ...query, page, signal: controller.signal });
      if (this.controller !== controller) return false;
      this.rows.value = result.traces ?? [];
      this.filters.value = query;
      this.state.value = { page: result.page, total: result.total, totalPages: result.total_pages };
      return true;
    } catch (error) {
      if (this.controller === controller && error.name !== 'AbortError') this.error.value = String(error.message ?? error);
      return false;
    } finally {
      if (this.controller === controller) {
        this.controller = null;
        this.loading.value = false;
      }
    }
  }

  refresh() {
    return this.load({ page: this.state.value.page || 1 });
  }

  next() {
    return this.go(this.state.value.page + 1);
  }

  go(page) {
    if (this.loading.value || !Number.isSafeInteger(page) || page < 1 || page > this.state.value.totalPages) return;
    return this.load({ page });
  }
}
