import { onBeforeUnmount, onMounted, ref, unref, watch } from 'vue';

import {
  panTimeViewport,
  wheelZoomFactor,
  zoomTimeViewport,
} from './model.js';

const HOLD_KEYS = new Set(['KeyW', 'KeyS', 'KeyA', 'KeyD']);
const INITIAL_PAN_STEP_PX = 50;
const INITIAL_ZOOM_STEP = 0.1;
const SNAP_FACTOR = 0.4;
const ACCELERATION_PER_MS = 1 / 50;
const DEFAULT_ANIMATION_DURATION = 700;
const ZOOM_RATIO_PER_FRAME = 0.008;
const KEYBOARD_PAN_PX_PER_FRAME = 8;

export function useTimelineNavigation({
  viewport,
  activeViewport,
  bounds,
  surfaceRef,
  trackRef,
  trackSelector,
  reset,
  setViewport,
  animationDriver,
}) {
  const panning = ref(false);
  const heldKeys = new Set();
  const driver = animationDriver ?? createBrowserAnimationDriver();
  let surfaceElement = null;
  let surfaceActive = false;
  let panState = null;
  let pendingWheelViewport = null;
  let animationRunning = false;
  let pointerAnchorRatio = 0.5;
  let keyboardTrackWidth = 0;
  let panDirection = 0;
  let panOffsetPx = 0;
  let targetPanOffsetPx = 0;
  let panAnimationStartMs = 0;
  let panAnimationEndMs = 0;
  let zoomDirection = 0;
  let zoomRatio = 0;
  let targetZoomRatio = 0;
  let zoomAnimationStartMs = 0;
  let zoomAnimationEndMs = 0;

  // The timeline can appear after an async model build or be replaced by v-if.
  watch(() => unref(surfaceRef), bindSurface, { immediate: true, flush: 'post' });

  onMounted(() => {
    globalThis.addEventListener('keydown', handleKeydown, true);
    globalThis.addEventListener('keyup', handleKeyup, true);
    globalThis.addEventListener('blur', stopKeyboardControl);
  });

  onBeforeUnmount(() => {
    bindSurface(null);
    globalThis.removeEventListener('keydown', handleKeydown, true);
    globalThis.removeEventListener('keyup', handleKeyup, true);
    globalThis.removeEventListener('blur', stopKeyboardControl);
  });

  function bindSurface(nextSurface) {
    if (nextSurface === surfaceElement) {
      return;
    }
    surfaceElement?.removeEventListener('pointerenter', activateSurface);
    surfaceElement?.removeEventListener('pointerleave', deactivateSurface);
    surfaceElement?.removeEventListener('pointerdown', activateSurface);
    surfaceElement?.removeEventListener('pointermove', capturePointerPosition);
    surfaceActive = false;
    stopKeyboardControl();
    panAnimationEndMs = 0;
    zoomAnimationEndMs = 0;
    pendingWheelViewport = null;
    pointerAnchorRatio = 0.5;
    keyboardTrackWidth = 0;
    stopPan(false);
    stopAnimation();
    surfaceElement = nextSurface;
    surfaceElement?.addEventListener('pointerenter', activateSurface);
    surfaceElement?.addEventListener('pointerleave', deactivateSurface);
    surfaceElement?.addEventListener('pointerdown', activateSurface);
    surfaceElement?.addEventListener('pointermove', capturePointerPosition);
  }

  function zoomBy(factor, anchorRatio = 0.5) {
    commitViewport(zoomTimeViewport(
      pendingWheelViewport ?? unref(activeViewport),
      unref(bounds),
      factor,
      anchorRatio,
    ));
    pendingWheelViewport = null;
  }

  function panBy(spanRatio) {
    const current = pendingWheelViewport ?? unref(activeViewport);
    commitViewport(panTimeViewport(
      current,
      unref(bounds),
      current.spanMs * spanRatio,
    ));
    pendingWheelViewport = null;
  }

  function resetViewport() {
    pendingWheelViewport = null;
    reset?.();
  }

  function commitViewport(nextViewport) {
    if (setViewport) {
      setViewport(nextViewport);
    } else {
      viewport.value = nextViewport;
    }
  }

  function handleKeydown(event) {
    if (
      !navigationIsActive(event.target)
      || event.metaKey
      || event.ctrlKey
      || event.altKey
      || event.isComposing
      || editableTarget(event.target)
    ) {
      return;
    }
    const code = keyCode(event);
    if (code === 'Digit0' || code === 'Numpad0') {
      consumeNavigationEvent(event);
      resetViewport();
      return;
    }
    if (!HOLD_KEYS.has(code)) {
      return;
    }
    consumeNavigationEvent(event);
    heldKeys.add(code);
    refreshTrackMetrics();
    updateKeyboardDirections(performance.now(), true);
    ensureAnimation();
  }

  function handleKeyup(event) {
    const code = keyCode(event);
    if (!heldKeys.has(code)) {
      return;
    }
    consumeNavigationEvent(event);
    heldKeys.delete(code);
    updateKeyboardDirections(performance.now(), false);
  }

  function updateKeyboardDirections(nowMs, extendAnimation) {
    const nextPanDirection = heldKeys.has('KeyA') === heldKeys.has('KeyD')
      ? 0
      : (heldKeys.has('KeyA') ? -1 : 1);
    const nextZoomDirection = heldKeys.has('KeyW') === heldKeys.has('KeyS')
      ? 0
      : (heldKeys.has('KeyW') ? 1 : -1);
    updatePanDirection(nextPanDirection, nowMs, extendAnimation);
    updateZoomDirection(nextZoomDirection, nowMs, extendAnimation);
  }

  function updatePanDirection(nextDirection, nowMs, extendAnimation) {
    const changed = nextDirection !== panDirection;
    if (changed && nextDirection !== 0) {
      panOffsetPx = 0;
      targetPanOffsetPx = nextDirection * INITIAL_PAN_STEP_PX;
      panAnimationStartMs = nowMs;
      panAnimationEndMs = nowMs + DEFAULT_ANIMATION_DURATION;
    }
    panDirection = nextDirection;
    if (nextDirection !== 0 && (extendAnimation || nowMs >= panAnimationEndMs)) {
      if (nowMs >= panAnimationEndMs) {
        panAnimationStartMs = nowMs;
      }
      panAnimationEndMs = nowMs + DEFAULT_ANIMATION_DURATION;
    }
  }

  function updateZoomDirection(nextDirection, nowMs, extendAnimation) {
    const changed = nextDirection !== zoomDirection;
    if (changed && nextDirection !== 0) {
      zoomRatio = 0;
      targetZoomRatio = nextDirection * INITIAL_ZOOM_STEP;
      zoomAnimationStartMs = nowMs;
      zoomAnimationEndMs = nowMs + DEFAULT_ANIMATION_DURATION;
    }
    zoomDirection = nextDirection;
    if (nextDirection !== 0 && (extendAnimation || nowMs >= zoomAnimationEndMs)) {
      if (nowMs >= zoomAnimationEndMs) {
        zoomAnimationStartMs = nowMs;
      }
      zoomAnimationEndMs = nowMs + DEFAULT_ANIMATION_DURATION;
    }
  }

  function runNavigationFrame(timestamp) {
    if (pendingWheelViewport) {
      const nextViewport = pendingWheelViewport;
      pendingWheelViewport = null;
      commitViewport(nextViewport);
    }
    if (panState?.dirty) {
      commitPan();
    }
    const keyboardAnimating = stepKeyboardNavigation(timestamp);
    if (!pendingWheelViewport && !panState?.dirty && !keyboardAnimating) {
      stopAnimation();
    }
  }

  function stepKeyboardNavigation(timestamp) {
    const panAnimating = stepKeyboardPan(timestamp);
    const zoomAnimating = stepKeyboardZoom(timestamp);
    return panAnimating || zoomAnimating;
  }

  function stepKeyboardPan(timestamp) {
    if (!panAnimationEndMs || timestamp >= panAnimationEndMs) {
      panAnimationEndMs = 0;
      return false;
    }
    const step = (targetPanOffsetPx - panOffsetPx) * SNAP_FACTOR;
    if (panDirection !== 0) {
      const elapsedMs = Math.max(Math.round(timestamp - panAnimationStartMs), 0);
      const velocity = 1 + elapsedMs * ACCELERATION_PER_MS;
      const targetStep = Math.max(KEYBOARD_PAN_PX_PER_FRAME * velocity, step);
      targetPanOffsetPx += panDirection * targetStep;
    }
    panOffsetPx += step;
    if (Math.abs(step) > 0.1) {
      panByPixels(step);
      return true;
    }
    panAnimationEndMs = 0;
    return false;
  }

  function stepKeyboardZoom(timestamp) {
    if (!zoomAnimationEndMs || timestamp >= zoomAnimationEndMs) {
      zoomAnimationEndMs = 0;
      return false;
    }
    const step = (targetZoomRatio - zoomRatio) * SNAP_FACTOR;
    if (zoomDirection !== 0) {
      const elapsedMs = Math.max(Math.round(timestamp - zoomAnimationStartMs), 0);
      const velocity = 1 + elapsedMs * ACCELERATION_PER_MS;
      const targetStep = Math.max(ZOOM_RATIO_PER_FRAME * velocity, step);
      targetZoomRatio += zoomDirection * targetStep;
    }
    zoomRatio += step;
    if (Math.abs(step) > 0.000001) {
      zoomBy(1 / (1 - step), pointerAnchorRatio);
      return true;
    }
    zoomAnimationEndMs = 0;
    return false;
  }

  function panByPixels(pixels) {
    const width = keyboardTrackWidth || refreshTrackMetrics();
    if (width > 0) {
      panBy(pixels / width);
    }
  }

  function stopKeyboardControl() {
    heldKeys.clear();
    updateKeyboardDirections(performance.now(), false);
  }

  function handleWheel(event) {
    if (!event.ctrlKey) {
      return;
    }
    if (trackSelector && !event.target.closest(trackSelector)) {
      return;
    }
    const rect = unref(trackRef)?.getBoundingClientRect();
    if (!rect?.width) {
      return;
    }
    event.preventDefault();
    const anchorRatio = clampRatio((event.clientX - rect.left) / rect.width);
    pendingWheelViewport = zoomTimeViewport(
      pendingWheelViewport ?? unref(activeViewport),
      unref(bounds),
      wheelZoomFactor(event.deltaY),
      anchorRatio,
    );
    ensureAnimation();
  }

  function startPan(event) {
    if (
      event.button !== 0
      || (trackSelector && !event.target.closest(trackSelector))
      || event.target.closest('button, a, input, textarea, select')
    ) {
      return;
    }
    const rect = unref(trackRef)?.getBoundingClientRect();
    if (!rect?.width) {
      return;
    }
    event.preventDefault();
    panState = {
      pointerId: event.pointerId,
      startX: event.clientX,
      viewport: { ...(pendingWheelViewport ?? unref(activeViewport)) },
      trackWidth: rect.width,
      clientX: event.clientX,
      dirty: false,
    };
    pendingWheelViewport = null;
    panning.value = true;
    globalThis.addEventListener('pointermove', movePan);
    globalThis.addEventListener('pointerup', stopPan);
    globalThis.addEventListener('pointercancel', stopPan);
  }

  function movePan(event) {
    if (!panState || event.pointerId !== panState.pointerId) {
      return;
    }
    panState.clientX = event.clientX;
    panState.dirty = true;
    ensureAnimation();
  }

  function commitPan() {
    if (!panState) {
      return;
    }
    panState.dirty = false;
    const deltaMs = -(
      (panState.clientX - panState.startX)
      / panState.trackWidth
    ) * panState.viewport.spanMs;
    commitViewport(panTimeViewport(panState.viewport, unref(bounds), deltaMs));
  }

  function stopPan(commit = true) {
    if (commit && panState?.dirty) {
      commitPan();
    }
    panState = null;
    panning.value = false;
    globalThis.removeEventListener('pointermove', movePan);
    globalThis.removeEventListener('pointerup', stopPan);
    globalThis.removeEventListener('pointercancel', stopPan);
  }

  function ensureAnimation() {
    if (animationRunning) {
      return;
    }
    animationRunning = true;
    driver.start(runNavigationFrame);
  }

  function stopAnimation() {
    if (!animationRunning) {
      return;
    }
    animationRunning = false;
    driver.stop(runNavigationFrame);
  }

  function capturePointerPosition(event) {
    const rect = unref(trackRef)?.getBoundingClientRect();
    if (!rect?.width) {
      return;
    }
    keyboardTrackWidth = rect.width;
    pointerAnchorRatio = clampRatio((event.clientX - rect.left) / rect.width);
  }

  function refreshTrackMetrics() {
    const width = unref(trackRef)?.getBoundingClientRect()?.width ?? 0;
    keyboardTrackWidth = width;
    return width;
  }

  function activateSurface() {
    surfaceActive = true;
  }

  function deactivateSurface() {
    surfaceActive = false;
    stopKeyboardControl();
  }

  function navigationIsActive(target) {
    const surface = unref(surfaceRef);
    return surfaceActive
      || Boolean(surface?.matches(':hover'))
      || (target instanceof Node && Boolean(surface?.contains(target)));
  }

  return {
    panning,
    zoomBy,
    panBy,
    resetViewport,
    handleWheel,
    startPan,
    stopPan,
  };
}

function createBrowserAnimationDriver() {
  const handles = new Map();
  return {
    start(callback) {
      if (handles.has(callback)) {
        return;
      }
      const tick = (timestamp) => {
        if (!handles.has(callback)) {
          return;
        }
        callback(timestamp);
        if (handles.has(callback)) {
          handles.set(callback, requestAnimationFrame(tick));
        }
      };
      handles.set(callback, requestAnimationFrame(tick));
    },
    stop(callback) {
      const handle = handles.get(callback);
      if (handle !== undefined) {
        cancelAnimationFrame(handle);
        handles.delete(callback);
      }
    },
  };
}

function editableTarget(target) {
  return target instanceof Element
    && Boolean(target.closest('input, textarea, select, [contenteditable="true"]'));
}

function keyCode(event) {
  if (event.code) {
    return event.code;
  }
  const key = String(event.key ?? '').toUpperCase();
  return key === '0' ? 'Digit0' : `Key${key}`;
}

function clampRatio(value) {
  return Math.min(Math.max(Number(value) || 0, 0), 1);
}

function consumeNavigationEvent(event) {
  event.preventDefault();
  event.stopPropagation();
  event.stopImmediatePropagation();
}
