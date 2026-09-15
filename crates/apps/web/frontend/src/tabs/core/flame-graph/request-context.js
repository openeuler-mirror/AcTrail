// Request interpretation used only by the flame graph. Stored actions are never rewritten.
export function requestContext(body) {
  const messages = Array.isArray(body.messages) ? body.messages
    : Array.isArray(body.input) ? body.input : [];
  const users = messages.filter((message) => ['user', 'human'].includes(message?.role)
    && !onlyToolResults(message.content));
  const latest = users.at(-1);
  const userTexts = textParts(latest?.content ?? (typeof body.input === 'string' ? body.input : body.prompt));
  const systemParts = [
    ...textParts(body.system),
    ...messages.filter((message) => ['system', 'developer'].includes(message?.role))
      .flatMap((message) => textParts(message.content)),
  ];
  const system = systemParts.join(' ').toLowerCase();
  const current = [...systemParts, ...textParts(body.prompt), ...userTexts].join(' ').toLowerCase();
  let backgroundKind = null;
  if ((system.includes('title generator')
      && ['thread title', 'title for this conversation', 'find this conversation later'].some((text) => system.includes(text)))
    || (system.includes('generate a concise, sentence-case title')
      && system.includes('coding session') && system.includes('single "title" field'))) {
    backgroundKind = 'title_generation';
  } else if ((current.includes('your task is to create a detailed summary of the conversation so far')
      && current.includes("paying close attention to the user's explicit requests"))
    || (current.includes('conversation history') && current.includes('about to be compacted') && current.includes('summary'))) {
    backgroundKind = 'context_compaction';
  } else if (system.includes('conversation summarizer')
    || (system.includes('summarize the conversation') && (system.includes('output only') || system.includes('return only')))) {
    backgroundKind = 'conversation_summary';
  }
  return { userTexts, backgroundKind };
}

function onlyToolResults(content) {
  return Array.isArray(content)
    ? content.length > 0 && content.every(toolResult)
    : toolResult(content);
}

function toolResult(block) {
  return ['tool_result', 'tool-result', 'function_call_output', 'tool_output'].includes(block?.type);
}

function textParts(content) {
  if (typeof content === 'string') return [content];
  if (Array.isArray(content)) return content.flatMap(textParts);
  if (content && ['text', 'input_text', 'output_text'].includes(content.type)
    && typeof content.text === 'string') return [content.text];
  return [];
}

export function toolCallArguments(toolCall, actionById) {
  const response = actionById.get(toolCall?.attributes?.['llm.tool_call.response_action_id']);
  try {
    const calls = JSON.parse(response?.attributes?.['llm.response.tool_calls_json'] ?? 'null');
    if (!Array.isArray(calls)) return null;
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

// Bound concurrent reads and retain only the small interpretation, not full histories.
export function createRequestContextLoader(readContent) {
  let cachedTrace = null;
  let cache = new Map();
  return async (traceId, actions, isCurrent = () => true) => {
    if (String(traceId) !== cachedTrace) {
      cachedTrace = String(traceId);
      cache = new Map();
    }
    const currentCache = cache;
    const requests = actions.filter((action) => action.kind === 'llm.request');
    const contexts = new Map();
    let cursor = 0;
    await Promise.all(Array.from({ length: Math.min(4, requests.length) }, async () => {
      while (isCurrent() && cursor < requests.length) {
        const request = requests[cursor++];
        const hash = request.attributes?.['llm.request.canonical_body_hash'];
        const key = `${request.id}\u0000${hash ?? ''}`;
        let context = currentCache.get(key);
        if (!context) {
          try {
            const { content } = await readContent(traceId, request.id, { maxBytes: 4 * 1024 * 1024 });
            if (!isCurrent()) return;
            if (!content || content.action_id !== request.id || content.truncated) continue;
            const body = JSON.parse(content.body_json);
            if (!body || typeof body !== 'object' || Array.isArray(body)) continue;
            context = requestContext(body);
            if (hash) currentCache.set(key, context);
          } catch {
            continue;
          }
        }
        contexts.set(request.id, context);
      }
    }));
    return { contexts, unavailable: requests.length - contexts.size };
  };
}
