import { semanticActionTarget } from '../actionLabels.js';

// Scoped to one projection; the source actions remain authoritative and untouched.
export class ToolCallDisplay {
  static statusLabel(action) {
    if (action?.kind !== 'llm.tool_call' && action?.kind !== 'llm.tool_result') {
      return action?.status;
    }
    const state = action.attributes?.['web.tool.result_state'];
    if (state === 'error' || (action.kind === 'llm.tool_result'
      && action.attributes?.['llm.tool_result.is_error'] === 'true')) {
      return 'Tool reported error';
    }
    if (state === 'returned' || action.kind === 'llm.tool_result') {
      return 'Result returned · execution outcome unconfirmed';
    }
    return 'Requested · result not observed';
  }

  constructor(actionById = new Map()) {
    this.actionById = actionById;
    this.callsByResponse = new Map();
  }

  arguments(toolCall) {
    const response = this.actionById.get(toolCall?.attributes?.['llm.tool_call.response_action_id']);
    if (!response) return null;
    try {
      if (!this.callsByResponse.has(response.id)) {
        const calls = JSON.parse(response.attributes?.['llm.response.tool_calls_json'] ?? 'null');
        this.callsByResponse.set(response.id, Array.isArray(calls) ? calls : []);
      }
      const calls = this.callsByResponse.get(response.id);
      const id = toolCall.attributes?.['llm.tool_call.id'];
      const ordinal = Number(toolCall.attributes?.['llm.tool_call.ordinal']);
      const call = (id ? calls.find((candidate) => candidate.id === id) : null)
        ?? (Number.isInteger(ordinal) ? calls[ordinal] : null);
      const declaration = call?.function ?? call;
      const value = declaration?.arguments_json ?? declaration?.arguments ?? declaration?.input;
      return typeof value === 'string' ? JSON.parse(value) : value ?? null;
    } catch {
      return null;
    }
  }

  target(action) {
    const observed = semanticActionTarget(action);
    if (observed || action?.kind !== 'llm.tool_call') return observed || '';
    const args = this.arguments(action);
    if (!args || typeof args !== 'object' || Array.isArray(args)) return '';
    const targets = ['command', 'file_path', 'filePath', 'path', 'paths', 'url']
      .flatMap((key) => Array.isArray(args[key]) ? args[key] : [args[key]])
      .filter((value) => typeof value === 'string' && value.trim());
    return [...new Set(targets)].join(' · ');
  }

  label(action) {
    const name = action?.attributes?.['llm.tool_call.name'] || 'tool.call';
    const target = this.target(action);
    return target ? `${name} · ${target}` : name;
  }
}
