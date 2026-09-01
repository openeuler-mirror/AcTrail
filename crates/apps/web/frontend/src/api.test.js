import test from 'node:test';
import assert from 'node:assert/strict';

import { readActionLlmRequestContentNode, readLlmTrajectoryGraph } from './api.js';

test('LLM request JSON pointers encode spaces without form-style plus signs', async () => {
  let requestedPath = '';
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (path) => {
    requestedPath = path;
    return {
      ok: true,
      json: async () => ({}),
    };
  };

  try {
    await readActionLlmRequestContentNode('trace-1', 'request-1', {
      pointer: '/foo bar+baz',
      offset: 0,
      limit: 50,
    });
  } finally {
    globalThis.fetch = originalFetch;
  }

  assert.match(requestedPath, /pointer=%2Ffoo%20bar%2Bbaz/);
  assert.doesNotMatch(requestedPath, /foo\+bar/);
});

test('trajectory graph request encodes the trace id and forwards the abort signal', async () => {
  let requestedPath = '';
  let requestedSignal = null;
  const controller = new AbortController();
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (path, options) => {
    requestedPath = path;
    requestedSignal = options.signal;
    return {
      ok: true,
      json: async () => ({ nodes: [], edges: [] }),
    };
  };

  try {
    await readLlmTrajectoryGraph('trace / 1', { signal: controller.signal });
  } finally {
    globalThis.fetch = originalFetch;
  }

  assert.equal(requestedPath, '/api/traces/trace%20%2F%201/llm-trajectories');
  assert.equal(requestedSignal, controller.signal);
});
