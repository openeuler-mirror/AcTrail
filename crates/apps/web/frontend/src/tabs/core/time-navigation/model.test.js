import assert from 'node:assert/strict';
import test from 'node:test';

import {
  constrainTimeViewport,
  panTimeViewport,
  projectTimeInterval,
  wheelZoomFactor,
  zoomTimeViewport,
} from './model.js';

test('projects and clips intervals into a viewport', () => {
  assert.deepEqual(projectTimeInterval(25, 75, { startMs: 50, spanMs: 100 }), {
    leftPct: 0,
    widthPct: 25,
  });
  assert.equal(projectTimeInterval(0, 10, { startMs: 50, spanMs: 100 }), null);
  assert.deepEqual(
    projectTimeInterval(
      25,
      40,
      { startMs: 50, spanMs: 100 },
      { startMs: 20, spanMs: 160 },
    ),
    { leftPct: -25, widthPct: 15 },
  );
});

test('keeps zoom anchored and constrains panning to trace bounds', () => {
  const bounds = { startMs: 0, spanMs: 1000 };
  assert.deepEqual(
    zoomTimeViewport({ startMs: 0, spanMs: 1000 }, bounds, 2, 0.5),
    { startMs: 250, spanMs: 500 },
  );
  assert.deepEqual(
    panTimeViewport({ startMs: 250, spanMs: 500 }, bounds, 900),
    { startMs: 500, spanMs: 500 },
  );
  assert.deepEqual(
    constrainTimeViewport({ startMs: -20, spanMs: 1200 }, bounds),
    bounds,
  );
});

test('normalizes wheel zoom logarithmically like Perfetto', () => {
  assert.equal(wheelZoomFactor(0), 1);
  assert.ok(wheelZoomFactor(-100) > 1);
  assert.ok(wheelZoomFactor(100) < 1);
  assert.ok(wheelZoomFactor(-1_000) < wheelZoomFactor(-10_000));
  assert.ok(wheelZoomFactor(-10_000) < 1.4);
});
