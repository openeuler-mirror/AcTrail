const FILE_ACTION_KINDS = new Set([
  'file.read',
  'file.write',
  'file.modify',
  'file.tty_io',
  'file.bulk_read',
  'fs.enumerate',
]);

export function semanticActionLabel(action) {
  if (action?.kind === 'command.invocation') {
    if (action.attributes?.['invocation.kind'] === 'agent') {
      return 'tool.call:agent.invoke';
    }
    return 'tool.call:bash.exec';
  }
  if (action?.kind === 'file.read') {
    return 'tool.call:file.read';
  }
  if (action?.kind === 'file.write') {
    return 'tool.call:file.write';
  }
  if (action?.kind === 'file.modify') {
    return 'tool.call:file.modify';
  }
  if (action?.kind === 'file.tty_io') {
    return 'tool.call:file.tty_io';
  }
  if (action?.kind === 'file.bulk_read') {
    return 'tool.call:file.bulk_read';
  }
  if (action?.kind === 'fs.enumerate') {
    return 'tool.call:fs.enumerate';
  }
  if (action?.kind === 'agent.invocation') {
    return 'tool.call:agent.invoke';
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
    return attributes['file.path'] ?? action.title;
  }
  if (action?.kind === 'agent.invocation') {
    return attributes['agent.child.command_line'] ?? attributes['agent.child.executable'] ?? action.title;
  }
  if (action?.kind === 'llm.call' || action?.kind === 'llm.request' || action?.kind === 'llm.response') {
    return attributes['llm.call.model'] ?? attributes['llm.request.model'] ?? attributes['llm.response.model'] ?? attributes.model;
  }
  return '';
}
