// Rebuild bounded waterfall data asynchronously while preserving the displayed model.
import { onBeforeUnmount, ref, shallowRef, watch } from 'vue';
import { buildWaterfall, emptyWaterfallModel } from '../model';

export class WaterfallModel {
  constructor(props, idle) {
    this.data = shallowRef(emptyWaterfallModel());
    this.building = ref(false);
    this.token = 0;
    this.handle = null;
    watch(() => [props.waterfall?.actions, props.waterfall?.links, idle.data.value.intervals,
      idle.data.value.axisEnd, props.timelineWindow, props.waterfall?.contextActions],
    ([actions, links, intervals, axisEnd, window, context]) => {
      this.schedule(actions, links, intervals, axisEnd, window, context);
    }, { immediate: true });
    onBeforeUnmount(() => this.cancel());
  }

  cancel() {
    this.token += 1;
    if (this.handle !== null) {
      if (typeof cancelIdleCallback === 'function') cancelIdleCallback(this.handle);
      else clearTimeout(this.handle);
      this.handle = null;
    }
  }

  schedule(actions, links, intervals, axisEnd, window, context) {
    this.cancel();
    const token = this.token;
    if (!actions?.length && !links?.length && !intervals?.length) {
      this.data.value = { ...emptyWaterfallModel(), ...(window ? { window } : {}) };
      this.building.value = false;
      return;
    }
    this.building.value = true;
    const build = () => {
      this.handle = null;
      if (token !== this.token) return;
      this.data.value = buildWaterfall(actions, links, intervals, axisEnd, context, window);
      this.building.value = false;
    };
    this.handle = typeof requestIdleCallback === 'function'
      ? requestIdleCallback(build, { timeout: 120 }) : setTimeout(build, 0);
  }
}
