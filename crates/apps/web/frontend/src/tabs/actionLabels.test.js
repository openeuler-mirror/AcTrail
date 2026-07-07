import assert from 'node:assert/strict';
import test from 'node:test';

import * as labels from './actionLabels.js';

test('labels MCP command invocations as MCP server commands', () => {
  assert.equal(
    labels.semanticActionLabel({
      kind: 'command.invocation',
      attributes: {
        'invocation.kind': 'mcp',
      },
    }),
    'tool.call:mcp_server',
  );
});

test('labels remote MCP client transport actions distinctly', () => {
  assert.equal(labels.semanticActionLabel({ kind: 'mcp.client_send' }), 'mcp.client_send');
  assert.equal(labels.semanticActionLabel({ kind: 'mcp.client_receive' }), 'mcp.client_receive');
});

test('targets remote MCP client transport actions like other MCP details', () => {
  assert.equal(
    labels.semanticActionTarget({
      kind: 'mcp.client_send',
      title: 'MCP client send',
      attributes: {
        'mcp.server.name': 'remote_probe',
        'mcp.tool.name': 'emit_remote_marker',
        'mcp.request.id': '81',
      },
    }),
    'remote_probe.emit_remote_marker #81',
  );
  assert.equal(
    labels.semanticActionTarget({
      kind: 'mcp.client_receive',
      title: 'MCP client receive',
      attributes: {
        'mcp.tool.name': 'remote_probe.emit_remote_marker',
        'mcp.request.id': '82',
      },
    }),
    'remote_probe.emit_remote_marker #82',
  );
});

test('MCP transport action meta distinguishes primary result from auxiliary ping request', () => {
  assert.equal(typeof labels.semanticActionMeta, 'function');
  assert.equal(
    labels.semanticActionMeta({
      kind: 'mcp.stdin',
      attributes: {
        'mcp.request.id': '3',
        'mcp.tool_call.request_id': '3',
        'mcp.message.id': '3',
        'mcp.exchange.index': '1',
      },
    }),
    'primary tools/call result',
  );
  assert.equal(
    labels.semanticActionMeta({
      kind: 'mcp.stdin',
      attributes: {
        'mcp.request.id': '3',
        'mcp.tool_call.request_id': '3',
        'mcp.message.id': 'server-ping-1',
        'mcp.message.method': 'ping',
        'mcp.exchange.index': '2',
      },
    }),
    'aux request ping',
  );
});

test('MCP transport action meta distinguishes primary request from auxiliary response', () => {
  assert.equal(
    labels.semanticActionMeta({
      kind: 'mcp.stdout',
      attributes: {
        'mcp.request.id': '3',
        'mcp.tool_call.request_id': '3',
        'mcp.message.id': '3',
        'mcp.message.method': 'tools/call',
        'mcp.exchange.index': '1',
      },
    }),
    'primary tools/call request',
  );
  assert.equal(
    labels.semanticActionMeta({
      kind: 'mcp.stdout',
      attributes: {
        'mcp.request.id': '3',
        'mcp.tool_call.request_id': '3',
        'mcp.message.id': 'server-ping-1',
        'mcp.exchange.index': '2',
      },
    }),
    'aux response',
  );
});
