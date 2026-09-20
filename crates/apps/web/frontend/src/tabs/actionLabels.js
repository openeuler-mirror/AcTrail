import { mcpActionMeta } from '../mcp/messageClassification.js';

const FILE_ACTION_KINDS = new Set([
  'file.read',
  'file.write',
  'file.modify',
  'file.tty_io',
  'file.bulk_read',
  'fs.enumerate',
]);

export function semanticActionLabel(action) {
  const attributes = action?.attributes ?? {};
  if (action?.kind === 'process.exec' && action.status === 'error') {
    const errno = Number(attributes.errno ?? attributes['syscall.result']);
    return Math.abs(errno) === 2 ? 'process.exec (not found)' : 'process.exec (failed)';
  }
  if (action?.kind === 'file.modify') {
    const operation = attributes['file.operation'];
    const label = operation ? `file.${operation}` : action.kind;
    if (action.status === 'error') {
      if (operation === 'mkdir' && Math.abs(Number(attributes.errno ?? attributes['syscall.result'])) === 17) {
        return `${label} (already exists)`;
      }
      return `${label} (failed)`;
    }
    if (attributes['file.open_intent'] === 'true') {
      return `${label} (write intent)`;
    }
    return label;
  }
  if (action?.kind === 'llm.tool_call') {
    const tool = attributes['llm.tool_call.name'];
    return tool ? `tool.call:${tool}` : action.kind;
  }
  if (action?.kind === 'mcp.tool_call') {
    return 'tool.call:mcp';
  }
  if (action?.kind === 'mcp.request') {
    return 'mcp.request';
  }
  if (action?.kind === 'mcp.response') {
    return 'mcp.response';
  }
  if (action?.kind === 'mcp.stdin') {
    return 'mcp.stdin';
  }
  if (action?.kind === 'mcp.stdout') {
    return 'mcp.stdout';
  }
  if (action?.kind === 'llm.call') {
    return 'llm.call';
  }
  return action?.kind ?? '';
}

export function isBashWrapperCommand(action) {
  const attributes = action?.attributes ?? {};
  const line =
    attributes['command.line'] ?? attributes['agent.child.command_line'] ?? action?.title ?? '';
  const executable = attributes['process.executable'] ?? '';
  const text = String(line).trim();
  const exe = String(executable).trim();
  if (!text && !exe) {
    return false;
  }
  const usesDashC = /\s-c(?:\s|$)/.test(text);
  if (usesDashC && (/(?:^|\/)bash(?:\s|$)/.test(text) || /\/bash$/.test(exe))) {
    return true;
  }
  return false;
}

export function semanticActionTarget(action) {
  const attributes = action?.attributes ?? {};
  if (action?.kind === 'command.invocation') {
    return attributes['agent.child.command_line'] ?? attributes['command.line'] ?? action.title;
  }
  if (FILE_ACTION_KINDS.has(action?.kind)) {
    if (attributes['file.operation'] === 'rename') {
      return `${attributes['file.path'] || 'path unavailable'} → ${attributes.target_path || 'path unavailable'}`;
    }
    if (attributes.path_resolution === 'missing') {
      return 'path unavailable';
    }
    return attributes['file.path'] ?? action.title;
  }
  if (action?.kind === 'llm.tool_call') {
    return attributes['file.path'] ?? attributes['command.line'] ?? '';
  }
  if (action?.kind === 'agent.invocation') {
    return attributes['agent.child.command_line'] ?? attributes['agent.child.executable'] ?? action.title;
  }
  if (
    action?.kind === 'mcp.tool_call'
    || action?.kind === 'mcp.request'
    || action?.kind === 'mcp.response'
    || action?.kind === 'mcp.stdin'
    || action?.kind === 'mcp.stdout'
  ) {
    const server = attributes['mcp.server.name'];
    const tool = attributes['mcp.tool.name'];
    const requestId = attributes['mcp.request.id'];
    const target = (() => {
      if (server && tool) {
        return String(tool).startsWith(`${server}.`) ? tool : `${server}.${tool}`;
      }
      return tool ?? server ?? action.title;
    })();
    if (action?.kind !== 'mcp.tool_call' && requestId) {
      return `${target} #${requestId}`;
    }
    return target;
  }
  if (action?.kind === 'llm.call' || action?.kind === 'llm.request' || action?.kind === 'llm.response') {
    return attributes['llm.call.model'] ?? attributes['llm.request.model'] ?? attributes['llm.response.model'] ?? attributes.model;
  }
  return '';
}

export function semanticActionMeta(action) {
  return mcpActionMeta(action);
}
