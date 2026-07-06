import test from 'node:test';
import assert from 'node:assert/strict';

import { buildMcpDetailInsight } from './insight.js';

test('buildMcpDetailInsight builds client stdout metadata and payload blocks', () => {
  const insight = buildMcpDetailInsight(
    detail('stdout-1', 'mcp.stdout', {
      'mcp.server.name': 'filesystem',
      'mcp.tool.name': 'read_file',
      'mcp.request.id': '7',
      'mcp.transport': 'stdio',
      'mcp.message.id': 'msg-1',
      'mcp.message.method': 'tools/call',
      'mcp.message.direction': 'client_to_server',
      'mcp.message.sequence': 12,
      'mcp.exchange.index': 3,
      'payload.stream_key': 'pipe:1',
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.request.action_id': 'request-1',
      'mcp.response.action_id': 'response-1',
    }),
    '{"jsonrpc":"2.0","id":7,"method":"tools/call"}',
  );

  assert.equal(insight.heading, 'MCP Client Stdout');
  assert.deepEqual(
    insight.chips.map((chip) => [chip.label, chip.value]),
    [
      ['server', 'filesystem'],
      ['tool', 'read_file'],
      ['request', '7'],
      ['method', 'tools/call'],
      ['direction', 'client_to_server'],
      ['transport', 'stdio'],
    ],
  );
  assert.equal(insight.blocks[0].id, 'mcp-perspective');
  assert.deepEqual(insight.blocks[0].rows, [
    ['protocol_view', 'mcp.request when the JSON-RPC message asks the server to run the tool'],
    ['client_process_view', 'mcp.stdout because the AI agent writes the bytes to the server'],
    ['server_process_view', 'server stdin because the MCP server reads the same bytes'],
  ]);
  assert.equal(insight.blocks[1].id, 'mcp-message');
  assert.ok(insight.blocks[1].rows.some(([key, value]) => key === 'exchange_index' && value === 3));
  assert.equal(insight.blocks[2].id, 'mcp-payload');
  assert.equal(insight.blocks[2].text, '{"jsonrpc":"2.0","id":7,"method":"tools/call"}');
});

test('buildMcpDetailInsight builds remote MCP request metadata without payload block', () => {
  const insight = buildMcpDetailInsight(
    detail('request-1', 'mcp.request', {
      'mcp.server.name': 'remote_probe',
      'mcp.tool.name': 'emit_remote_marker',
      'mcp.request.id': '81',
      'mcp.transport': 'streamable_http',
      'http.request.method': 'POST',
      'server.address': 'remote.example.test',
      'url.path': '/mcp',
      'payload.stream_key': 'remote-mcp',
      'mcp.tool_call.action_id': 'tool-1',
    }),
    '{"jsonrpc":"2.0","id":81,"method":"tools/call"}',
  );

  assert.equal(insight.heading, 'MCP Request');
  assert.deepEqual(insight.blocks[0].rows, [
    ['protocol_view', 'mcp.request when the JSON-RPC message asks the server to run the tool'],
    ['transport_view', 'streamable_http HTTP request payload'],
  ]);
  assert.equal(insight.blocks[1].id, 'mcp-message');
  assert.equal(insight.blocks[1].label, 'Protocol summary');
  assert.deepEqual(insight.blocks[1].rows, [
    ['tool', 'emit_remote_marker'],
    ['request_id', '81'],
    ['transport', 'streamable_http'],
  ]);
  assert.equal(insight.blocks.some((block) => block.id === 'mcp-payload'), false);
});

test('buildMcpDetailInsight builds remote MCP response protocol summary without payload block', () => {
  const insight = buildMcpDetailInsight(
    detail('response-1', 'mcp.response', {
      'mcp.server.name': 'remote_probe',
      'mcp.tool.name': 'emit_remote_marker',
      'mcp.request.id': '82',
      'mcp.transport': 'streamable_http',
      'mcp.execution.status': 'success',
      'payload.stream_key': 'remote-mcp',
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.stdin.action_id': 'stdin-1',
    }),
    '{"jsonrpc":"2.0","id":82,"result":{"content":[],"isError":false}}',
  );

  assert.equal(insight.heading, 'MCP Response');
  assert.deepEqual(insight.blocks[0].rows, [
    ['protocol_view', 'mcp.response when the JSON-RPC message returns the tool result'],
    ['transport_view', 'streamable_http HTTP response payload'],
  ]);
  assert.equal(insight.blocks[1].id, 'mcp-message');
  assert.equal(insight.blocks[1].label, 'Protocol summary');
  assert.equal(insight.blocks[1].title, 'tools/call protocol response');
  assert.deepEqual(insight.blocks[1].rows, [
    ['tool', 'emit_remote_marker'],
    ['request_id', '82'],
    ['transport', 'streamable_http'],
    ['execution_status', 'success'],
  ]);
  assert.equal(insight.blocks.some((block) => block.id === 'mcp-payload'), false);
});

test('buildMcpDetailInsight labels primary MCP stdin result as JSON-RPC message', () => {
  const insight = buildMcpDetailInsight(
    detail('stdin-result-1', 'mcp.stdin', {
      'mcp.server.name': 'filesystem',
      'mcp.tool.name': 'read_file',
      'mcp.request.id': '3',
      'mcp.tool_call.request_id': '3',
      'mcp.message.id': '3',
      'mcp.message.direction': 'inbound',
      'mcp.exchange.index': '1',
    }),
    '{"jsonrpc":"2.0","id":3,"result":{"content":[]}}',
  );

  assert.equal(insight.blocks[1].label, 'JSON-RPC message');
  assert.equal(insight.blocks[1].title, 'primary tools/call response');
  assert.deepEqual(insight.blocks[1].rows, [
    ['tools_call_id', '3'],
    ['message_id', '3'],
    ['exchange_index', '1'],
    ['direction', 'inbound'],
  ]);
});

test('buildMcpDetailInsight labels auxiliary MCP stdin ping as JSON-RPC request', () => {
  const insight = buildMcpDetailInsight(
    detail('stdin-ping-1', 'mcp.stdin', {
      'mcp.server.name': 'filesystem',
      'mcp.tool.name': 'read_file',
      'mcp.request.id': '3',
      'mcp.tool_call.request_id': '3',
      'mcp.message.id': 'server-ping-1',
      'mcp.message.method': 'ping',
      'mcp.message.direction': 'inbound',
      'mcp.exchange.index': '2',
    }),
    '{"jsonrpc":"2.0","id":"server-ping-1","method":"ping"}',
  );

  assert.equal(insight.blocks[1].label, 'JSON-RPC message');
  assert.equal(insight.blocks[1].title, 'auxiliary JSON-RPC request: ping');
  assert.deepEqual(insight.blocks[1].rows, [
    ['tools_call_id', '3'],
    ['message_id', 'server-ping-1'],
    ['method', 'ping'],
    ['exchange_index', '2'],
    ['direction', 'inbound'],
  ]);
});

test('buildMcpDetailInsight builds remote MCP client send metadata and payload blocks', () => {
  const insight = buildMcpDetailInsight(
    detail('client-send-1', 'mcp.client_send', {
      'mcp.server.name': 'remote_probe',
      'mcp.tool.name': 'emit_remote_marker',
      'mcp.request.id': '83',
      'mcp.transport': 'streamable_http',
      'mcp.tool_call.request_id': '83',
      'mcp.message.id': '83',
      'mcp.message.method': 'tools/call',
      'mcp.message.direction': 'outbound',
      'mcp.exchange.index': '1',
      'http.request.method': 'POST',
      'server.address': 'remote.example.test',
      'url.path': '/mcp',
      'payload.stream_key': 'remote-mcp',
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.request.action_id': 'request-1',
    }),
    '{"jsonrpc":"2.0","id":83,"method":"tools/call"}',
  );

  assert.equal(insight.heading, 'MCP Client Send');
  assert.deepEqual(insight.blocks[0].rows, [
    ['protocol_view', 'mcp.request when the JSON-RPC message asks the server to run the tool'],
    ['transport_view', 'mcp.client_send because the client sends the HTTP request payload'],
  ]);
  assert.equal(insight.blocks[1].label, 'JSON-RPC message');
  assert.equal(insight.blocks[1].title, 'primary tools/call request');
  assert.deepEqual(insight.blocks[1].rows, [
    ['tools_call_id', '83'],
    ['message_id', '83'],
    ['method', 'tools/call'],
    ['exchange_index', '1'],
    ['direction', 'outbound'],
  ]);
  assert.equal(insight.blocks[2].title, 'captured client send JSON-RPC');
});

test('buildMcpDetailInsight ignores non-MCP stdio details', () => {
  assert.equal(buildMcpDetailInsight(detail('http-1', 'http.message', {}), ''), null);
});

function detail(id, kind, attributes) {
  return {
    raw: {
      id,
      kind,
      attributes,
    },
  };
}
