import { ref, shallowRef } from 'vue';
import { readActionDetail } from '../../../../api';

export class WaterfallDetail {
  constructor() {
    this.id = ref(null);
    this.data = shallowRef(null);
    this.error = ref('');
    this.controller = null;
  }

  clear() {
    this.controller?.abort();
    this.controller = null;
    this.id.value = null;
    this.data.value = null;
    this.error.value = '';
  }

  async select(traceId, action, present) {
    this.clear();
    const controller = new AbortController();
    this.controller = controller;
    this.id.value = action.id;
    this.data.value = present(action);
    try {
      const full = await readActionDetail(traceId, action.id, { signal: controller.signal });
      if (this.controller !== controller) return;
      if (full.id !== action.id) throw new Error('Action detail does not match the selection');
      this.data.value = present(full);
    } catch (error) {
      if (this.controller === controller && error.name !== 'AbortError') {
        this.error.value = String(error.message ?? error);
      }
    }
  }

}
