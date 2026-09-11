const animationCallbacks = new Set();
const registeredDraws = new Set();
const pendingDraws = new Set();
let frameHandle = null;
let redrawAll = false;
let flushing = false;

export const flameAnimationDriver = Object.freeze({
  start: startFlameAnimation,
  stop: stopFlameAnimation,
});

export function registerFlameDraw(draw) {
  registeredDraws.add(draw);
  return () => {
    registeredDraws.delete(draw);
    pendingDraws.delete(draw);
  };
}

export function scheduleFlameFrame(draw) {
  pendingDraws.add(draw);
  ensureFlameFrame();
}

export function scheduleFlameRedraw() {
  redrawAll = true;
  ensureFlameFrame();
}

export function cancelFlameFrame(draw) {
  pendingDraws.delete(draw);
}

export function startFlameAnimation(callback) {
  animationCallbacks.add(callback);
  ensureFlameFrame();
}

export function stopFlameAnimation(callback) {
  animationCallbacks.delete(callback);
}

function ensureFlameFrame() {
  if (!flushing && frameHandle === null) {
    frameHandle = requestAnimationFrame(flushFlameFrame);
  }
}

function flushFlameFrame(timestamp) {
  frameHandle = null;
  flushing = true;
  for (const callback of [...animationCallbacks]) {
    callback(timestamp);
  }

  const draws = redrawAll ? [...registeredDraws] : [...pendingDraws];
  redrawAll = false;
  pendingDraws.clear();
  for (const draw of draws) {
    draw();
  }
  flushing = false;

  if (animationCallbacks.size || redrawAll || pendingDraws.size) {
    ensureFlameFrame();
  }
}
