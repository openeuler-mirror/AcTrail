const PERFETTO_WHEEL_ZOOM_SPEED = -0.02;

export function projectTimeInterval(
  intervalStartMs,
  intervalEndMs,
  viewport,
  visibilityViewport = viewport,
) {
  const viewportStart = Number(viewport?.startMs);
  const viewportSpan = Number(viewport?.spanMs);
  const visibilityStart = Number(visibilityViewport?.startMs);
  const visibilitySpan = Number(visibilityViewport?.spanMs);
  if (
    !Number.isFinite(viewportStart)
    || !Number.isFinite(viewportSpan)
    || viewportSpan <= 0
    || !Number.isFinite(visibilityStart)
    || !Number.isFinite(visibilitySpan)
    || visibilitySpan <= 0
  ) {
    return null;
  }
  const visibilityEnd = visibilityStart + visibilitySpan;
  const visibleStart = Math.max(Number(intervalStartMs), visibilityStart);
  const visibleEnd = Math.min(Number(intervalEndMs), visibilityEnd);
  if (!Number.isFinite(visibleStart) || !Number.isFinite(visibleEnd) || visibleEnd <= visibleStart) {
    return null;
  }
  return {
    leftPct: ((visibleStart - viewportStart) / viewportSpan) * 100,
    widthPct: ((visibleEnd - visibleStart) / viewportSpan) * 100,
  };
}

export function constrainTimeViewport(viewport, bounds, minimumSpanMs = 0.001) {
  const boundsStart = Number(bounds?.startMs) || 0;
  const boundsSpan = Math.max(Number(bounds?.spanMs) || minimumSpanMs, minimumSpanMs);
  const spanMs = Math.min(
    Math.max(Number(viewport?.spanMs) || boundsSpan, minimumSpanMs),
    boundsSpan,
  );
  const latestStart = boundsStart + boundsSpan - spanMs;
  const startMs = Math.min(
    Math.max(Number(viewport?.startMs) || boundsStart, boundsStart),
    latestStart,
  );
  return { startMs, spanMs };
}

export function zoomTimeViewport(viewport, bounds, factor, anchorRatio = 0.5) {
  const current = constrainTimeViewport(viewport, bounds);
  const safeFactor = Number.isFinite(factor) && factor > 0 ? factor : 1;
  const anchor = Math.min(Math.max(Number(anchorRatio) || 0, 0), 1);
  const minimumSpanMs = Math.max(Math.min(Number(bounds?.spanMs) * 0.000001, 1), 0.001);
  const nextSpan = current.spanMs / safeFactor;
  const anchorTime = current.startMs + current.spanMs * anchor;
  return constrainTimeViewport({
    startMs: anchorTime - nextSpan * anchor,
    spanMs: nextSpan,
  }, bounds, minimumSpanMs);
}

export function panTimeViewport(viewport, bounds, deltaMs) {
  const current = constrainTimeViewport(viewport, bounds);
  return constrainTimeViewport({
    startMs: current.startMs + Number(deltaMs || 0),
    spanMs: current.spanMs,
  }, bounds);
}

export function wheelZoomFactor(deltaY) {
  const numericDelta = Number(deltaY) || 0;
  if (!numericDelta) {
    return 1;
  }
  const sign = numericDelta < 0 ? -1 : 1;
  const normalizedDelta = sign * Math.log2(1 + Math.abs(numericDelta));
  const spanScale = 1 - normalizedDelta * PERFETTO_WHEEL_ZOOM_SPEED;
  return spanScale > 0 ? 1 / spanScale : 1;
}
