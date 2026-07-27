import test from 'node:test';
import assert from 'node:assert/strict';

import { buildLlmDetailInsight } from './insight.js';

test('LLM request content restores message context and available tools', () => {
  const insight = buildLlmDetailInsight(
    {
      raw: {
        id: 'request-1',
        kind: 'llm.request',
        title: 'LLM request',
        attributes: {},
      },
    },
    {
      action_id: 'request-1',
      body_json: JSON.stringify({
        messages: [
          { role: 'system', content: 'Be concise.' },
          { role: 'user', content: 'Check this repository.' },
        ],
        tools: [
          {
            type: 'function',
            function: {
              name: 'read_file',
              description: 'Read one file.',
              parameters: { type: 'object' },
            },
          },
        ],
      }),
    },
  );

  assert.equal(insight.chips.find((item) => item.label === 'messages')?.value, '2');
  assert.equal(insight.chips.find((item) => item.label === 'tools')?.value, '1');
  assert.equal(insight.blocks.find((block) => block.id === 'message-context')?.items.length, 2);
  assert.equal(insight.blocks.find((block) => block.id === 'request-tools')?.items[0].title, 'read_file');
});

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
