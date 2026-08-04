import { classifyMcpMessage } from './messageClassification.js';

const MCP_REQUEST_KIND = 'mcp.request';
const MCP_RESPONSE_KIND = 'mcp.response';
const MCP_STDIN_KIND = 'mcp.stdin';
const MCP_STDOUT_KIND = 'mcp.stdout';
const MCP_ENVELOPE_VIEW_KINDS = new Set([MCP_STDIN_KIND, MCP_STDOUT_KIND]);
const MCP_STDIO_CONTENT_LINKS = Object.freeze({
  [MCP_STDIN_KIND]: Object.freeze({
    jsonRpcRole: 'response',
    actionIdAttribute: 'mcp.response.action_id',
  }),
  [MCP_STDOUT_KIND]: Object.freeze({
    jsonRpcRole: 'request',
    actionIdAttribute: 'mcp.request.action_id',
  }),
});

export function hasMcpJsonRpcContent(kind) {
  return kind === MCP_REQUEST_KIND || kind === MCP_RESPONSE_KIND;
}

export function mcpJsonRpcContentSource(action) {
  const kind = action?.kind;
  if (!kind) {
    return null;
  }
  if (hasMcpJsonRpcContent(kind)) {
    return {
      actionId: requiredActionId(action, 'id'),
      viewKind: kind,
    };
  }
  const link = MCP_STDIO_CONTENT_LINKS[kind];
  if (!link) {
    return null;
  }
  const classification = classifyMcpMessage(action);
  if (!classification.isPrimaryToolsCall) {
    return null;
  }
  if (classification.jsonRpcRole !== link.jsonRpcRole) {
    throw new Error(
      `${kind} primary tools/call must be a JSON-RPC ${link.jsonRpcRole}, got ${String(classification.jsonRpcRole)}`,
    );
  }
  return {
    actionId: requiredActionId(action.attributes, link.actionIdAttribute),
    viewKind: kind,
  };
}

export function deriveMcpJsonRpcView(kind, content) {
  if (!hasMcpJsonRpcContent(kind) && !MCP_ENVELOPE_VIEW_KINDS.has(kind)) {
    throw new Error(`MCP semantic content is not defined for ${String(kind)}`);
  }
  if (!content) {
    throw new Error(`canonical MCP JSON-RPC content is not retained for ${kind}`);
  }
  if (content.truncated) {
    throw new Error(
      `canonical MCP JSON-RPC exceeds the requested ${content.returned_bytes} byte view; request a larger max_bytes value`,
    );
  }
  const envelope = JSON.parse(content.canonical_json);
  if (!envelope || Array.isArray(envelope) || typeof envelope !== 'object') {
    throw new Error('canonical MCP JSON-RPC content must be an object');
  }
  if (MCP_ENVELOPE_VIEW_KINDS.has(kind)) {
    return {
      label: 'JSON-RPC',
      title: 'canonical envelope',
      text: prettyJson(envelope),
    };
  }
  if (kind === MCP_REQUEST_KIND) {
    if (!Object.hasOwn(envelope, 'params')) {
      throw new Error('canonical MCP request has no params member');
    }
    return {
      label: 'Payload',
      title: 'params',
      text: prettyJson(envelope.params),
    };
  }
  if (Object.hasOwn(envelope, 'error')) {
    return {
      label: 'Payload',
      title: 'error',
      text: prettyJson(envelope.error),
    };
  }
  if (!Object.hasOwn(envelope, 'result')) {
    throw new Error('canonical MCP response has neither error nor result');
  }
  return {
    label: 'Payload',
    title: 'result',
    text: prettyJson(envelope.result),
  };
}

function requiredActionId(source, key) {
  const value = source?.[key];
  if (value === undefined || value === null || value === '') {
    throw new Error(`MCP canonical content source is missing ${key}`);
  }
  return String(value);
}

function prettyJson(value) {
  return JSON.stringify(value, null, 2);
}
