// Track presentation timing only for IDs retained by the current bounded track.
export class ActionEntrance {
  constructor(redraw) {
    this.ids = new Set();
    this.started = new Map();
    this.motion = globalThis.matchMedia?.('(prefers-reduced-motion: reduce)');
    this.onMotionChange = () => {
      this.started.clear();
      redraw();
    };
    this.motion?.addEventListener('change', this.onMotionChange);
  }

  update(activities, enabled, durationMs) {
    durationMs = Number.isFinite(durationMs) ? Math.max(durationMs, 0) : 0;
    this.durationMs = durationMs;
    const next = new Set(activities.filter(activity => !activity.synthetic).map(activity => activity.id));
    const animate = enabled && !this.motion?.matches && durationMs > 0;
    if (!animate) this.started.clear();
    const now = performance.now();
    for (const id of this.started.keys()) {
      if (!next.has(id) || now - this.started.get(id) >= durationMs) this.started.delete(id);
    }
    if (animate) {
      for (const id of next) {
        if (!this.ids.has(id)) this.started.set(id, now);
      }
    }
    this.ids = next;
  }

  opacity(id, now) {
    const start = this.started.get(id);
    if (start == null) return 1;
    const progress = Math.min(Math.max((now - start) / this.durationMs, 0), 1);
    if (progress === 1) this.started.delete(id);
    return 1 - (1 - progress) ** 3;
  }

  dispose() {
    this.motion?.removeEventListener('change', this.onMotionChange);
    this.ids.clear();
    this.started.clear();
  }
}
