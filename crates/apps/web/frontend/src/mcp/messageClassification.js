const MCP_REQUEST_KIND = 'mcp.request';
const MCP_RESPONSE_KIND = 'mcp.response';
const MCP_STDIN_KIND = 'mcp.stdin';
const MCP_STDOUT_KIND = 'mcp.stdout';
const MCP_CLIENT_SEND_KIND = 'mcp.client_send';
const MCP_CLIENT_RECEIVE_KIND = 'mcp.client_receive';

const PROTOCOL_SUMMARY_KINDS = new Set([MCP_REQUEST_KIND, MCP_RESPONSE_KIND]);
const TRANSPORT_MESSAGE_KINDS = new Set([
  MCP_STDIN_KIND,
  MCP_STDOUT_KIND,
  MCP_CLIENT_SEND_KIND,
  MCP_CLIENT_RECEIVE_KIND,
]);

const REQUEST_KINDS = new Set([MCP_REQUEST_KIND, MCP_STDOUT_KIND, MCP_CLIENT_SEND_KIND]);
const RESPONSE_KINDS = new Set([MCP_RESPONSE_KIND, MCP_STDIN_KIND, MCP_CLIENT_RECEIVE_KIND]);

export function classifyMcpMessage(actionOrKind, attrsArg = null) {
  const action = typeof actionOrKind === 'object' && actionOrKind !== null ? actionOrKind : null;
  const kind = action?.kind ?? actionOrKind;
  const attrs = attrsArg ?? action?.attributes ?? {};
  const messageId = attrs['mcp.message.id'];
  const method = attrs['mcp.message.method'];
  const exchangeIndex = attrs['mcp.exchange.index'];
  const toolCallId = attrs['mcp.tool_call.request_id'] ?? attrs['mcp.request.id'];
  const jsonRpcRole = jsonRpcRoleFor(kind, attrs);
  const primaryToolsCall =
    TRANSPORT_MESSAGE_KINDS.has(kind) &&
    present(messageId) &&
    present(toolCallId) &&
    String(messageId) === String(toolCallId) &&
    String(exchangeIndex) === '1';

  return {
    protocolRole: protocolRoleFor(kind),
    isProtocolSummary: PROTOCOL_SUMMARY_KINDS.has(kind),
    isTransportMessage: TRANSPORT_MESSAGE_KINDS.has(kind),
    jsonRpcRole,
    method,
    toolCallId,
    isPrimaryToolsCall: primaryToolsCall,
  };
}

export function mcpProtocolSummaryTitle(classification) {
  return `tools/call protocol ${classification.protocolRole ?? 'message'}`;
}

export function mcpJsonRpcMessageTitle(classification) {
  if (classification.isPrimaryToolsCall) {
    return `primary tools/call ${classification.jsonRpcRole ?? 'message'}`;
  }
  if (classification.jsonRpcRole === 'request') {
    const method = classification.method ? `: ${classification.method}` : '';
    return `auxiliary JSON-RPC request${method}`;
  }
  if (classification.jsonRpcRole === 'response') {
    return 'auxiliary JSON-RPC response';
  }
  return 'JSON-RPC message';
}

export function mcpActionMeta(action) {
  const classification = classifyMcpMessage(action);
  if (!classification.isTransportMessage || !classification.jsonRpcRole) {
    return '';
  }
  if (classification.isPrimaryToolsCall) {
    return classification.jsonRpcRole === 'response'
      ? 'primary tools/call result'
      : 'primary tools/call request';
  }
  if (classification.jsonRpcRole === 'request') {
    return classification.method ? `aux request ${classification.method}` : 'aux request';
  }
  return 'aux response';
}

export function mcpJsonRpcPairKey(attrs) {
  const toolCallActionId = attrs['mcp.tool_call.action_id'];
  const toolCallId = attrs['mcp.tool_call.request_id'] ?? attrs['mcp.request.id'];
  if (!present(toolCallActionId) || !present(toolCallId)) {
    return null;
  }
  const base = `${toolCallActionId}\u0000${toolCallId}`;
  if (present(attrs['mcp.message.id'])) {
    return `${base}\u0000message:${attrs['mcp.message.id']}`;
  }
  if (present(attrs['mcp.exchange.index'])) {
    return `${base}\u0000exchange:${attrs['mcp.exchange.index']}`;
  }
  return `${base}\u0000single`;
}

function jsonRpcRoleFor(kind, attrs) {
  if (!TRANSPORT_MESSAGE_KINDS.has(kind)) {
    return null;
  }
  if (present(attrs['mcp.message.method'])) {
    return 'request';
  }
  if (present(attrs['mcp.message.id'])) {
    return 'response';
  }
  return null;
}

function protocolRoleFor(kind) {
  if (REQUEST_KINDS.has(kind)) {
    return 'request';
  }
  if (RESPONSE_KINDS.has(kind)) {
    return 'response';
  }
  return null;
}

function present(value) {
  return value !== undefined && value !== null && value !== '';
}
