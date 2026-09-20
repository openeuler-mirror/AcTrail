/** Derive delegation edges from recorded content for one trajectory view. */
export class OfflineAgentCorrelation {
  constructor(api, { concurrency = 4, maxBytes = 8 * 1024 * 1024 } = {}) {
    for (const [name, value] of Object.entries({ concurrency, maxBytes })) {
      if (!Number.isSafeInteger(value) || value <= 0) {
        throw new Error(`Offline correlation ${name} must be a positive integer`);
      }
    }
    this.api = api;
    this.concurrency = concurrency;
    this.maxBytes = maxBytes;
  }

  async derive(traceKey, graph, signal) {
    const nodes = new Map((graph.nodes ?? []).map((node) => [node.id, node]));
    const result = { edges: [], unavailable: 0 };
    if (!nodes.size) return result;
    const waterfall = await this.api.readWaterfall(traceKey, { signal });
    const actions = new Map((waterfall.actions ?? []).map((action) => [action.id, action]));
    const calls = new Map();
    for (const link of waterfall.links ?? []) {
      if (!link.valid || actions.get(link.parent)?.kind !== 'llm.call') continue;
      const field = link.role === 'llm.call.request' ? 'requests'
        : link.role === 'llm.call.response' ? 'responses' : null;
      if (!field) continue;
      const call = calls.get(link.parent) ?? { requests: new Set(), responses: new Set() };
      call[field].add(link.child);
      calls.set(link.parent, call);
    }
    const parents = new Map();
    for (const call of calls.values()) {
      if (call.requests.size !== 1 || call.responses.size !== 1) continue;
      const request = [...call.requests][0];
      const response = [...call.responses][0];
      if (!nodes.has(request) || actions.get(response)?.kind !== 'llm.response') continue;
      const ids = parents.get(response) ?? new Set();
      ids.add(request);
      parents.set(response, ids);
    }
    const invocations = [...actions.values()].filter((action) => action.kind === 'agent.invocation');
    const prompts = new Map();
    await this.each(invocations, signal, async (invocation) => {
      try {
        const callId = invocation.attributes?.['agent.invocation.tool_call_action_id'];
        if (!callId) return;
        const call = await this.api.readActionDetail(traceKey, callId, { signal });
        const responseId = call.attributes?.['llm.tool_call.response_action_id'];
        const ordinalText = call.attributes?.['llm.tool_call.ordinal'];
        const ordinal = Number(ordinalText);
        const parentIds = parents.get(responseId);
        if (parentIds?.size !== 1 || ordinalText == null || !Number.isInteger(ordinal) || ordinal < 0) {
          throw new Error('Invocation source unavailable');
        }
        const response = actions.get(responseId);
        const calls = JSON.parse(response?.attributes?.['llm.response.tool_calls_json'] ?? 'null');
        if (!Array.isArray(calls)) throw new Error('Tool call content unavailable');
        const declared = calls[ordinal];
        let argumentsValue = declared?.function?.arguments;
        if (typeof argumentsValue === 'string') argumentsValue = JSON.parse(argumentsValue);
        const prompt = argumentsValue?.prompt;
        if (typeof prompt !== 'string' || !prompt.trim()) return;
        const source = [...parentIds][0];
        const candidates = prompts.get(prompt.trim()) ?? [];
        candidates.push({
          source, invocation: invocation.id,
          start_time_unix_nanos: invocation.start_time_unix_nanos,
          targets: new Set(),
        });
        prompts.set(prompt.trim(), candidates);
      } catch (error) {
        signal?.throwIfAborted();
        result.unavailable += 1;
      }
    });
    if (!prompts.size) return result;
    const continuations = new Set((graph.edges ?? [])
      .filter((edge) => edge.kind === 'append' || edge.kind === 'fork')
      .map((edge) => edge.target));
    const roots = [...nodes.values()].filter((node) =>
      node.trajectory_position === 0 &&
      !continuations.has(node.id) &&
      node.completeness === 'complete' &&
      !actions.get(node.id)?.attributes?.['llm.request.background_kind'],
    );
    const matches = new Map();
    await this.each(roots, signal, async (node) => {
      try {
        const document = await this.api.readRequestContent(traceKey, node.id, {
          maxBytes: this.maxBytes, signal,
        });
        if (document.content?.truncated !== false) throw new Error('Request content unavailable');
        const body = JSON.parse(document.content.body_json);
        const parts = this.userText(body);
        const keys = new Set(parts);
        if (parts.length > 1) keys.add(parts.join('\n\n'));
        const candidates = new Set();
        for (const key of keys) {
          for (const candidate of prompts.get(key) ?? []) {
            const parent = nodes.get(candidate.source);
            if (parent.trajectory_id === node.trajectory_id ||
                !this.precedes(candidate, node)) continue;
            candidates.add(candidate);
          }
        }
        for (const candidate of candidates) candidate.targets.add(node.id);
        matches.set(node.id, candidates);
      } catch (error) {
        signal?.throwIfAborted();
        result.unavailable += 1;
      }
    });
    for (const [target, candidates] of matches) {
      if (candidates.size !== 1) continue;
      const candidate = [...candidates][0];
      if (candidate.targets.size !== 1) continue;
      result.edges.push({
        source: candidate.source, target, kind: 'delegation',
        confidence: 'derived', invocation_id: candidate.invocation,
      });
    }
    result.edges.sort((a, b) => a.target.localeCompare(b.target));
    return result;
  }

  async each(items, signal, visit) {
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(this.concurrency, items.length) }, async () => {
      while (next < items.length) {
        signal?.throwIfAborted();
        const item = items[next++];
        await visit(item);
      }
    }));
  }

  precedes(parent, child) {
    const left = parent.start_time_unix_nanos;
    const right = child.start_time_unix_nanos;
    return /^\d+$/.test(left) && /^\d+$/.test(right) && BigInt(left) < BigInt(right);
  }

  userText(body) {
    for (const messages of [body?.messages, body?.input]) {
      if (!Array.isArray(messages)) continue;
      for (let index = messages.length - 1; index >= 0; index -= 1) {
        const message = messages[index];
        if (!['user', 'human'].includes(message?.role)) continue;
        const content = message.content ?? message;
        const blocks = Array.isArray(content) ? content : [content];
        if (blocks.length && blocks.every((block) => this.isToolResult(block))) continue;
        const parts = [...this.text(content)];
        if (parts.length) return parts;
      }
    }
    const input = [...this.text(body?.input)];
    return input.length ? input : [...this.text(body?.prompt)];
  }

  isToolResult(block) {
    return ['tool_result', 'tool-result', 'function_call_output', 'tool_output'].includes(block?.type);
  }

  *text(value) {
    if (typeof value === 'string') {
      if (value.trim()) yield value.trim();
    } else if (Array.isArray(value)) {
      for (const item of value) yield* this.text(item);
    } else if (value && typeof value === 'object') {
      for (const key of ['text', 'content', 'input']) {
        if (value[key] != null) yield* this.text(value[key]);
      }
    }
  }
}
