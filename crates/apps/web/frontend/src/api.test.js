import test from 'node:test';
import assert from 'node:assert/strict';

import { readActionLlmRequestContentNode } from './api.js';

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
