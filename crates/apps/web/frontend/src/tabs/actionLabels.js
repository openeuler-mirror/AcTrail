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
  if (action?.kind === 'agent.invocation') {
    return 'tool.call:agent.invoke';
  }
  if (action?.kind === 'llm.call') {
    return 'llm.call';
  }
  return action?.kind ?? '';
}

export function semanticActionTarget(action) {
  const attributes = action?.attributes ?? {};
  if (action?.kind === 'command.invocation') {
    return attributes['agent.child.command_line'] ?? attributes['command.line'] ?? action.title;
  }
  if (action?.kind === 'file.read' || action?.kind === 'file.write' || action?.kind === 'file.modify') {
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
