import test from 'node:test';
import assert from 'node:assert/strict';

import { buildLlmDetailInsight } from './insight.js';

test('LLM response tool call without arguments does not render fake call text', () => {
  const insight = buildLlmDetailInsight({
    raw: {
      id: 'response-1',
      kind: 'llm.response',
      title: 'LLM response',
      attributes: {
        'llm.response.model': 'Qwen3.7-Plus',
        'llm.response.tool_calls_json': JSON.stringify([
          {
            id: 'call_abc',
            type: 'function',
            function: {
              name: 'bash',
            },
          },
        ]),
      },
    },
  });

  assert.equal(insight.blocks[0].id, 'tool-calls');
  assert.equal(insight.blocks[0].items[0].title, 'bash #1');
  assert.equal(insight.blocks[0].items[0].text, '');
});
