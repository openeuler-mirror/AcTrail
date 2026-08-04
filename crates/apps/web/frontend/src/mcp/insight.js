import { chip, compactChips, compactRows } from '../detail/insight.js';
import {
  classifyMcpMessage,
  mcpJsonRpcMessageTitle,
  mcpProtocolSummaryTitle,
} from './messageClassification.js';

const MCP_DETAIL_KINDS = new Set([
  'mcp.stdin',
  'mcp.stdout',
  'mcp.request',
  'mcp.response',
]);

export function buildMcpDetailInsight(detail, payload = '') {
  const action = detail?.raw ?? null;
  if (!action || !MCP_DETAIL_KINDS.has(action.kind)) {
    return null;
  }
  const attrs = action.attributes ?? {};
  const context = mcpDetailContext(action.kind);
  const classification = classifyMcpMessage(action.kind, attrs);
  const blocks = [
    perspectiveBlock(context),
    messageBlock(attrs, classification),
    payloadBlock({ payload, context }),
  ].filter(Boolean);

  return {
    instanceId: action.id,
    kind: action.kind,
    heading: context.heading,
    chips: compactChips([
      chip('server', attrs['mcp.server.name']),
      chip('tool', attrs['mcp.tool.name']),
      chip('request', attrs['mcp.request.id']),
      chip('method', attrs['mcp.message.method']),
      chip('direction', attrs['mcp.message.direction']),
      chip('transport', attrs['mcp.transport']),
    ]),
    blocks,
  };
}

function mcpDetailContext(kind) {
  if (kind === 'mcp.stdin') {
    return {
      heading: 'MCP Client Stdin',
      protocolRole: 'response',
      payloadLabel: 'JSON-RPC',
      payloadTitle: 'canonical envelope',
    };
  }
  if (kind === 'mcp.stdout') {
    return {
      heading: 'MCP Client Stdout',
      protocolRole: 'request',
      payloadLabel: 'JSON-RPC',
      payloadTitle: 'canonical envelope',
    };
  }
  if (kind === 'mcp.response') {
    return {
      heading: 'MCP Response',
      protocolRole: 'response',
      payloadLabel: 'Payload',
      payloadTitle: 'result',
    };
  }
  return {
    heading: 'MCP Request',
    protocolRole: 'request',
    payloadLabel: 'Payload',
    payloadTitle: 'params',
  };
}

function perspectiveBlock(context) {
  return {
    id: 'mcp-perspective',
    tone: 'tools',
    label: 'Perspective',
    title: 'protocol role vs process stdio',
    rows: context.protocolRole === 'response'
      ? [
          ['protocol_view', 'mcp.response returns the tool result'],
          ['client_process_view', 'mcp.stdin because the AI agent reads the server bytes'],
          ['server_process_view', 'server stdout writes the same bytes'],
        ]
      : [
          ['protocol_view', 'mcp.request asks the server to run the tool'],
          ['client_process_view', 'mcp.stdout because the AI agent writes the request bytes'],
          ['server_process_view', 'server stdin reads the same bytes'],
        ],
  };
}

function messageBlock(attrs, classification) {
  const rows = classification.isProtocolSummary
    ? protocolSummaryRows(attrs)
    : jsonRpcMessageRows(attrs, classification);
  if (!rows.length) {
    return null;
  }
  return {
    id: 'mcp-message',
    tone: 'tools',
    label: classification.isProtocolSummary ? 'Protocol summary' : 'JSON-RPC message',
    title: classification.isProtocolSummary
      ? mcpProtocolSummaryTitle(classification)
      : mcpJsonRpcMessageTitle(classification),
    rows,
  };
}

function protocolSummaryRows(attrs) {
  return compactRows({
    tool: attrs['mcp.tool.name'],
    request_id: attrs['mcp.request.id'],
    transport: attrs['mcp.transport'],
    execution_status: attrs['mcp.execution.status'],
  });
}

function jsonRpcMessageRows(attrs, classification) {
  return compactRows({
    tools_call_id: classification.toolCallId,
    message_id: attrs['mcp.message.id'],
    method: attrs['mcp.message.method'],
    exchange_index: attrs['mcp.exchange.index'],
    direction: attrs['mcp.message.direction'],
  });
}

function payloadBlock({ payload, context }) {
  const view = payload && typeof payload === 'object' ? payload : { text: payload };
  const text = String(view.text ?? '').trim();
  if (!text) {
    return null;
  }
  return {
    id: 'mcp-payload',
    tone: 'context',
    label: view.label ?? context.payloadLabel,
    title: view.title ?? context.payloadTitle,
    text,
  };
}
