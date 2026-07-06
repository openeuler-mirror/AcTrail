import { chip, compactChips, compactRows } from '../detail/insight.js';
import {
  classifyMcpMessage,
  mcpJsonRpcMessageTitle,
  mcpProtocolSummaryTitle,
} from './messageClassification.js';

const MCP_STDIN_KIND = 'mcp.stdin';
const MCP_STDOUT_KIND = 'mcp.stdout';
const MCP_REQUEST_KIND = 'mcp.request';
const MCP_RESPONSE_KIND = 'mcp.response';
const MCP_CLIENT_SEND_KIND = 'mcp.client_send';
const MCP_CLIENT_RECEIVE_KIND = 'mcp.client_receive';

export function buildMcpDetailInsight(detail, payloadText = '') {
  const action = detail?.raw ?? null;
  if (!action || !isMcpDetailKind(action.kind)) {
    return null;
  }
  const attrs = action.attributes ?? {};
  const context = mcpDetailContext(action.kind, attrs);
  const classification = classifyMcpMessage(action.kind, attrs);
  const blocks = [
    perspectiveBlock(context, attrs),
    messageBlock(attrs, classification),
    payloadBlock({ payloadText, context }),
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

function isMcpDetailKind(kind) {
  return (
    kind === MCP_STDIN_KIND ||
    kind === MCP_STDOUT_KIND ||
    kind === MCP_REQUEST_KIND ||
    kind === MCP_RESPONSE_KIND ||
    kind === MCP_CLIENT_SEND_KIND ||
    kind === MCP_CLIENT_RECEIVE_KIND
  );
}

function mcpDetailContext(kind) {
  if (kind === MCP_CLIENT_SEND_KIND) {
    return {
      heading: 'MCP Client Send',
      protocolRole: 'request',
      payloadTitle: 'captured client send JSON-RPC',
      payloadBearing: true,
      stdio: false,
      remoteClient: true,
    };
  }
  if (kind === MCP_CLIENT_RECEIVE_KIND) {
    return {
      heading: 'MCP Client Receive',
      protocolRole: 'response',
      payloadTitle: 'captured client receive JSON-RPC',
      payloadBearing: true,
      stdio: false,
      remoteClient: true,
    };
  }
  if (kind === MCP_STDIN_KIND) {
    return {
      heading: 'MCP Client Stdin',
      protocolRole: 'response',
      payloadTitle: 'captured client stdin JSON-RPC',
      payloadBearing: true,
      stdio: true,
    };
  }
  if (kind === MCP_STDOUT_KIND) {
    return {
      heading: 'MCP Client Stdout',
      protocolRole: 'request',
      payloadTitle: 'captured client stdout JSON-RPC',
      payloadBearing: true,
      stdio: true,
    };
  }
  if (kind === MCP_RESPONSE_KIND) {
    return {
      heading: 'MCP Response',
      protocolRole: 'response',
      payloadBearing: false,
      stdio: false,
    };
  }
  return {
    heading: 'MCP Request',
    protocolRole: 'request',
    payloadBearing: false,
    stdio: false,
  };
}

function perspectiveBlock(context, attrs) {
  if (context.remoteClient) {
    return {
      id: 'mcp-perspective',
      tone: 'tools',
      label: 'Perspective',
      title: 'protocol role vs remote transport',
      rows:
        context.protocolRole === 'response'
          ? [
              ['protocol_view', 'mcp.response when the JSON-RPC message returns the tool result'],
              ['transport_view', 'mcp.client_receive because the client receives the HTTP response payload'],
            ]
          : [
              ['protocol_view', 'mcp.request when the JSON-RPC message asks the server to run the tool'],
              ['transport_view', 'mcp.client_send because the client sends the HTTP request payload'],
            ],
    };
  }
  if (!context.stdio) {
    return {
      id: 'mcp-perspective',
      tone: 'tools',
      label: 'Perspective',
      title: 'protocol role vs transport',
      rows:
        context.protocolRole === 'response'
          ? [
              ['protocol_view', 'mcp.response when the JSON-RPC message returns the tool result'],
              ['transport_view', `${attrs['mcp.transport'] ?? 'transport'} HTTP response payload`],
            ]
          : [
              ['protocol_view', 'mcp.request when the JSON-RPC message asks the server to run the tool'],
              ['transport_view', `${attrs['mcp.transport'] ?? 'transport'} HTTP request payload`],
            ],
    };
  }
  return {
    id: 'mcp-perspective',
    tone: 'tools',
    label: 'Perspective',
    title: 'protocol role vs process stdio',
    rows: context.protocolRole === 'response'
      ? [
          ['protocol_view', 'mcp.response when the JSON-RPC message returns the tool result'],
          ['client_process_view', 'mcp.stdin because the AI agent reads the bytes from the server'],
          ['server_process_view', 'server stdout because the MCP server writes the same bytes'],
        ]
      : [
          ['protocol_view', 'mcp.request when the JSON-RPC message asks the server to run the tool'],
          ['client_process_view', 'mcp.stdout because the AI agent writes the bytes to the server'],
          ['server_process_view', 'server stdin because the MCP server reads the same bytes'],
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

function payloadBlock({ payloadText, context }) {
  if (!context.payloadBearing) {
    return null;
  }
  const text = String(payloadText ?? '').trim();
  if (!text) {
    return null;
  }
  return {
    id: 'mcp-payload',
    tone: 'context',
    label: 'Payload',
    title: context.payloadTitle,
    text,
  };
}
