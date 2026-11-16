// Build-time overrides are documented alongside the live-follow controller.
class FollowSettings {
  static read(env) {
    const defaults = { intervalMs: 1000, motionMs: 1000, entranceMs: 450, requestTimeoutMs: 10000,
      scrollThresholdMs: 30000, settleMs: 1 };
    const keys = { intervalMs: 'INTERVAL_MS', motionMs: 'MOTION_MS', entranceMs: 'ENTRANCE_MS',
      requestTimeoutMs: 'REQUEST_TIMEOUT_MS', scrollThresholdMs: 'SCROLL_THRESHOLD_MS', settleMs: 'SETTLE_MS' };
    for (const [name, suffix] of Object.entries(keys)) {
      const key = `VITE_FLAME_FOLLOW_${suffix}`;
      if (env[key] !== undefined) defaults[name] = Number(env[key]);
      if (!Number.isSafeInteger(defaults[name]) || defaults[name] <= 0) throw new Error(`Invalid ${key}: expected a positive integer`);
    }
    const idleTailRatio = Number(env.VITE_FLAME_FOLLOW_IDLE_TAIL_RATIO ?? 0.2);
    if (!Number.isFinite(idleTailRatio) || idleTailRatio <= 0 || idleTailRatio >= 1) {
      throw new Error('Invalid VITE_FLAME_FOLLOW_IDLE_TAIL_RATIO: expected a number between 0 and 1');
    }
    return Object.freeze({ ...defaults, idleTailRatio });
  }
}

export const FOLLOW_SETTINGS = FollowSettings.read(import.meta.env);
