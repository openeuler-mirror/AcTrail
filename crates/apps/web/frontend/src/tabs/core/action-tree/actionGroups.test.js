import test from 'node:test';
import assert from 'node:assert/strict';

import { groupActionNodes } from './actionGroups.js';

test('groupActionNodes keeps MCP transport messages individually visible', () => {
  const nodes = [
    action('client-receive-1', 'mcp.client_receive'),
    action('client-receive-2', 'mcp.client_receive'),
  ];

  const grouped = groupActionNodes(nodes);

  assert.deepEqual(grouped.map((node) => node.id), ['client-receive-1', 'client-receive-2']);
});

test('groupActionNodes does not merge command invocations with different semantic labels', () => {
  const nodes = [
    action('bash-wrapper', 'command.invocation', 'tool.call:bash.exec'),
    action('mcp-server', 'command.invocation', 'tool.call:mcp_server'),
  ];

  const grouped = groupActionNodes(nodes);

  assert.deepEqual(grouped.map((node) => node.id), ['bash-wrapper', 'mcp-server']);
});

test('groupActionNodes merges command invocations with the same semantic label', () => {
  const nodes = [
    action('bash-1', 'command.invocation', 'tool.call:bash.exec'),
    action('bash-2', 'command.invocation', 'tool.call:bash.exec'),
  ];

  const grouped = groupActionNodes(nodes);

  assert.equal(grouped.length, 1);
  assert.equal(grouped[0].semanticLabel, 'tool.call:bash.exec (2)');
});

function action(id, kind, semanticLabel = kind) {
  return {
    id,
    nodeType: 'action',
    kind,
    semanticLabel,
    title: semanticLabel,
    children: [],
  };
}
