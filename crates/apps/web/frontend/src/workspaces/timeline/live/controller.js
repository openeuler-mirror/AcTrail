import { ref } from 'vue';
import { FOLLOW_SETTINGS } from './settings.js';

// The timeline owns data and coordinates; this controller owns the follow lifecycle.
export class TimelineFollow {
  constructor(timeline, readMetadata, animationDriver, options = FOLLOW_SETTINGS) {
    this.timeline = timeline;
    this.readMetadata = readMetadata;
    this.driver = animationDriver;
    this.options = options;
    this.enabled = ref(false);
    this.timer = null;
    this.request = null;
    this.animation = null;
    this.frame = this.step.bind(this);
    this.visibility = () => {
      this.pause();
      if (this.enabled.value && !document.hidden) this.poll();
    };
    document.addEventListener('visibilitychange', this.visibility);
  }

  setEnabled(enabled) {
    this.pause();
    this.enabled.value = Boolean(enabled && this.timeline.traceId && this.timeline.window.value);
    if (!this.enabled.value) return;
    this.timeline.cancel();
    if (!document.hidden) this.poll();
  }

  pause() {
    clearTimeout(this.timer);
    this.timer = null;
    this.request?.abort();
    this.request = null;
    this.driver.stop(this.frame);
    // Data already belongs to the target window: leave a populated view on stop.
    if (this.animation) {
      this.timeline.viewport.value = this.animation.target;
      this.timeline.updateStatus();
    }
    this.animation = null;
  }

  dispose() {
    this.setEnabled(false);
    document.removeEventListener('visibilitychange', this.visibility);
  }

  async poll() {
    if (!this.enabled.value || document.hidden || this.request) return;
    const request = new AbortController();
    this.request = request;
    const timeout = setTimeout(() => request.abort(), this.options.requestTimeoutMs);
    try {
      const target = await this.timeline.refreshTail(this.readMetadata, this.options.scrollThresholdMs, request.signal);
      if (this.request === request && !request.signal.aborted && target) this.moveTo(target);
    } catch (error) {
      if (this.request === request) console.warn('Timeline follow refresh failed; retrying next poll', error);
    } finally {
      clearTimeout(timeout);
      if (this.request === request) {
        this.request = null;
        this.timer = setTimeout(() => this.poll(), this.options.intervalMs);
      }
    }
  }

  moveTo(target) {
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      this.driver.stop(this.frame);
      this.animation = null;
      this.timeline.viewport.value = target;
      this.timeline.updateStatus();
      return;
    }
    if (this.animation) {
      // Retarget without restarting the motion or discarding its velocity.
      this.animation.target = target;
      return;
    }
    const current = this.timeline.viewport.value;
    if (current.startMs === target.startMs && current.spanMs === target.spanMs) return;
    this.animation = { target, lastFrame: performance.now(), velocity: { startMs: 0, spanMs: 0 } };
    this.driver.start(this.frame);
  }

  step(now) {
    if (!this.animation) return;
    const { target, velocity, lastFrame } = this.animation;
    const elapsed = Math.max(now - lastFrame, 0);
    this.animation.lastFrame = now;
    // Exact critically damped motion stays stable across different frame rates.
    const rate = 6 / this.options.motionMs;
    const decay = Math.exp(-rate * elapsed);
    const next = {};
    let settled = true;
    for (const key of ['startMs', 'spanMs']) {
      const offset = this.timeline.viewport.value[key] - target[key];
      const change = velocity[key] + rate * offset;
      next[key] = target[key] + (offset + change * elapsed) * decay;
      velocity[key] = (velocity[key] - rate * change * elapsed) * decay;
      settled &&= Math.abs(next[key] - target[key]) <= this.options.settleMs
        && Math.abs(velocity[key]) <= this.options.settleMs / this.options.motionMs;
    }
    this.timeline.viewport.value = settled ? target : next;
    if (settled) {
      this.driver.stop(this.frame);
      this.animation = null;
      this.timeline.updateStatus();
    }
  }
}
