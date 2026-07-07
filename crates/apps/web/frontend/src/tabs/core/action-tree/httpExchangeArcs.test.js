import test from 'node:test';
import assert from 'node:assert/strict';

import { buildMessagePairArcOverlay } from './httpExchangeArcs.js';

test('buildMessagePairArcOverlay preserves HTTP request-response arcs', () => {
  const root = tree([
    action('http-request', 'http.message', {
      direction: 'outbound',
      'http.operation': 'request',
    }),
    action('http-response', 'http.message', {
      direction: 'inbound',
      'http.operation': 'response',
      'http.request.action_id': 'http-request',
    }),
  ]);
  const overlay = buildMessagePairArcOverlay(root, canvasFor(['http-request', 'http-response']));

  assert.equal(overlay.arcs.length, 1);
  assert.equal(overlay.arcs[0].id, 'http-request->http-response');
  assert.equal(overlay.arcs[0].className, 'http-exchange-arc');
});

test('buildMessagePairArcOverlay draws primary and auxiliary MCP arcs in JSON-RPC flow order', () => {
  const root = tree([
    action('stdout-tools-call', 'mcp.stdout', {
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.request.id': '42',
      'mcp.tool_call.request_id': '42',
      'mcp.message.id': '42',
      'mcp.message.method': 'tools/call',
      'mcp.exchange.index': 1,
    }),
    action('stdin-tools-result', 'mcp.stdin', {
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.request.id': '42',
      'mcp.tool_call.request_id': '42',
      'mcp.message.id': '42',
      'mcp.exchange.index': 1,
    }),
    action('stdin-ping', 'mcp.stdin', {
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.request.id': '42',
      'mcp.tool_call.request_id': '42',
      'mcp.message.id': 'server-ping-1',
      'mcp.message.method': 'ping',
      'mcp.exchange.index': 2,
    }),
    action('stdout-ping-response', 'mcp.stdout', {
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.request.id': '42',
      'mcp.tool_call.request_id': '42',
      'mcp.message.id': 'server-ping-1',
      'mcp.exchange.index': 2,
    }),
  ]);
  const overlay = buildMessagePairArcOverlay(
    root,
    canvasFor(['stdout-tools-call', 'stdin-tools-result', 'stdin-ping', 'stdout-ping-response']),
  );

  assert.deepEqual(
    overlay.arcs.map((arc) => ({ id: arc.id, className: arc.className })),
    [
      { id: 'stdout-tools-call->stdin-tools-result', className: 'mcp-exchange-arc' },
      { id: 'stdin-ping->stdout-ping-response', className: 'mcp-exchange-arc' },
    ],
  );
});

test('buildMessagePairArcOverlay falls back to MCP message id and only draws visible pairs', () => {
  const root = tree([
    action('stdin-visible', 'mcp.stdin', {
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.request.id': '99',
      'mcp.tool_call.request_id': '99',
      'mcp.message.id': 'msg-a',
    }),
    action('stdout-visible', 'mcp.stdout', {
      'mcp.tool_call.action_id': 'tool-1',
      'mcp.request.id': '99',
      'mcp.tool_call.request_id': '99',
      'mcp.message.id': 'msg-a',
      'mcp.message.method': 'ping',
    }),
    action('stdin-hidden-peer', 'mcp.stdin', {
      'mcp.tool_call.action_id': 'tool-2',
      'mcp.request.id': '100',
      'mcp.tool_call.request_id': '100',
      'mcp.message.id': 'msg-b',
    }),
    action('stdout-hidden-peer', 'mcp.stdout', {
      'mcp.tool_call.action_id': 'tool-2',
      'mcp.request.id': '100',
      'mcp.tool_call.request_id': '100',
      'mcp.message.id': 'msg-b',
      'mcp.message.method': 'ping',
    }),
  ]);
  const overlay = buildMessagePairArcOverlay(root, canvasFor(['stdin-visible', 'stdout-visible', 'stdin-hidden-peer']));

  assert.deepEqual(
    overlay.arcs.map((arc) => arc.id),
    ['stdout-visible->stdin-visible'],
  );
});

test('buildMessagePairArcOverlay pairs visible remote MCP client send and receive actions', () => {
  const root = tree([
    action('client-send-1', 'mcp.client_send', {
      'mcp.tool_call.action_id': 'tool-remote',
      'mcp.request.id': '81',
      'mcp.tool_call.request_id': '81',
      'mcp.message.id': '81',
      'mcp.message.method': 'tools/call',
      'mcp.exchange.index': 1,
    }),
    action('client-receive-1', 'mcp.client_receive', {
      'mcp.tool_call.action_id': 'tool-remote',
      'mcp.request.id': '81',
      'mcp.tool_call.request_id': '81',
      'mcp.message.id': '81',
      'mcp.exchange.index': 1,
    }),
    action('client-receive-2', 'mcp.client_receive', {
      'mcp.tool_call.action_id': 'tool-remote',
      'mcp.request.id': '81',
      'mcp.tool_call.request_id': '81',
      'mcp.message.id': '82',
      'mcp.exchange.index': 2,
    }),
  ]);
  const overlay = buildMessagePairArcOverlay(
    root,
    canvasFor(['client-send-1', 'client-receive-1', 'client-receive-2']),
  );

  assert.deepEqual(
    overlay.arcs.map((arc) => ({ id: arc.id, className: arc.className })),
    [{ id: 'client-send-1->client-receive-1', className: 'mcp-exchange-arc' }],
  );
});

function tree(children) {
  return {
    nodeType: 'agent',
    children,
  };
}

function action(id, kind, attributes) {
  return {
    id,
    kind,
    nodeType: 'action',
    detail: {
      raw: {
        id,
        kind,
        attributes,
      },
    },
    children: [],
  };
}

function canvasFor(ids) {
  const elements = ids.map((id, index) => elementFor(id, index));
  return {
    scrollWidth: 500,
    clientWidth: 500,
    scrollHeight: 500,
    clientHeight: 500,
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
    querySelectorAll: (selector) => (selector === '[data-action-node-id]' ? elements : []),
  };
}

function elementFor(id, index) {
  const rect = {
    left: 20 + index * 80,
    right: 70 + index * 80,
    top: 20 + index * 70,
    bottom: 60 + index * 70,
    width: 50,
    height: 40,
  };
  return {
    dataset: { actionNodeId: id },
    querySelector: () => null,
    getBoundingClientRect: () => rect,
  };
}
