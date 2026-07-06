import { chip, compactChips, compactRows, firstPresent, previewText } from '../detail/insight.js';

const USER_MESSAGE_ROLES = ['user', 'human'];
const ASSISTANT_MESSAGE_ROLES = ['assistant'];
const EMPTY_ARRAY = Object.freeze([]);
const MESSAGE_CONTEXT_DEFAULT_LIMIT = 6;
const TOOL_LIST_DEFAULT_LIMIT = 8;

export { previewText } from '../detail/insight.js';

export function buildLlmDetailInsight(detail, requestContent = null) {
  const action = detail?.raw ?? null;
  if (!action?.kind) {
    return null;
  }
  if (action.kind === 'llm.request') {
    return requestInsight(action, requestContent);
  }
  if (action.kind === 'llm.response') {
    return responseInsight(action);
  }
  if (action.kind === 'llm.call') {
    return callInsight(action);
  }
  return null;
}

export function buildLlmMessages(requestAction, responseAction, requestOverride = null, responseOverride = null) {
  const requestFull = requestOverride ?? llmRequestMessage(requestAction, { preview: false });
  const responseFull = responseOverride ?? llmResponseMessage(responseAction);
  if (!requestFull && !responseFull) {
    return null;
  }
  const requestPreview =
    requestOverride !== null
      ? previewText(requestOverride, 160)
      : previewText(llmRequestMessage(requestAction, { preview: true }) || requestFull, 160);
  const model =
    requestAction?.attributes?.['llm.request.model'] ??
    responseAction?.attributes?.['llm.response.model'] ??
    null;
  return {
    model,
    requestFull,
    responseFull,
    requestPreview,
    responsePreview: previewText(responseFull, 160),
  };
}

export function llmRequestMessage(action, { preview = false } = {}) {
  if (!action) {
    return '';
  }
  const attrs = action.attributes ?? {};
  const raw = attrs['llm.request.message_preview'] || '';
  return extractLlmRequestMessage(raw, preview);
}

export function llmResponseMessage(action) {
  if (!action) {
    return '';
  }
  const attrs = action.attributes ?? {};
  const parts = [
    attrs['llm.response.reasoning_text'],
    attrs['llm.response.content_text'],
  ].filter((value) => String(value ?? '').trim().length > 0);
  if (parts.length > 0) {
    return Array.from(new Set(parts)).join('\n\n');
  }
  return '';
}

export function extractLlmRequestMessage(raw, preview = false) {
  const text = String(raw ?? '').trim();
  if (!text) {
    return '';
  }
  if (looksLikeJson(text)) {
    const parsed = parseJson(text);
    if (parsed !== null) {
      if (preview) {
        const userText = messagesTextByRoles(parsed?.messages, USER_MESSAGE_ROLES);
        if (userText) {
          return userText;
        }
        if (typeof parsed?.input === 'string') {
          return parsed.input.trim();
        }
        if (typeof parsed?.prompt === 'string') {
          return parsed.prompt.trim();
        }
      }
      const fromMessages = messagesText(parsed?.messages ?? parsed?.input);
      if (fromMessages) {
        return fromMessages;
      }
      if (typeof parsed?.prompt === 'string') {
        return parsed.prompt.trim();
      }
      if (typeof parsed?.input === 'string') {
        return parsed.input.trim();
      }
    }
  }
  return text;
}

export function extractLlmAssistantMessage(raw) {
  const text = String(raw ?? '').trim();
  if (!text) {
    return '';
  }
  if (looksLikeJson(text)) {
    const parsed = parseJson(text);
    if (parsed !== null) {
      const fromChoices = openAiChoicesText(parsed);
      if (fromChoices) {
        return fromChoices;
      }
      const fromOutput = openAiResponsesOutputText(parsed);
      if (fromOutput) {
        return fromOutput;
      }
      const fromAnthropic = anthropicResponseText(parsed);
      if (fromAnthropic) {
        return fromAnthropic;
      }
      const fromMessages = messagesTextByRoles(parsed?.messages, ASSISTANT_MESSAGE_ROLES);
      if (fromMessages) {
        return fromMessages;
      }
    }
  }
  return text;
}

function requestInsight(action, requestContent) {
  const attrs = action.attributes ?? {};
  const body = requestBodyFromContent(action, requestContent);
  const messages = requestMessages(body);
  const newestMessages = messages.slice().reverse();
  const tools = requestTools(body);
  const lastMessage = messages.filter((message) => message.text).at(-1) ?? null;
  const fallbackMessage = llmRequestMessage(action, { preview: false });
  const blocks = [];
  if (lastMessage) {
    blocks.push({
      id: 'last-message',
      tone: 'request',
      label: 'Last message',
      title: messageTitle(lastMessage),
      text: lastMessage.text,
    });
  } else if (fallbackMessage) {
    blocks.push({
      id: 'message-preview',
      tone: 'request',
      label: 'Message preview',
      title: 'captured preview',
      text: fallbackMessage,
    });
  }
  if (messages.length > 0) {
    blocks.push({
      id: 'message-context',
      tone: 'context',
      label: 'Message context',
      title: `${messages.length} message block${messages.length === 1 ? '' : 's'}`,
      itemLimit: MESSAGE_CONTEXT_DEFAULT_LIMIT,
      items: newestMessages.map((message, index) => ({
        id: `${message.index}-${index}`,
        title: messageTitle(message),
        text: message.text,
      })),
    });
  }
  if (tools.length > 0) {
    blocks.push({
      id: 'request-tools',
      tone: 'tools',
      label: 'Available tools',
      title: `${tools.length} tool${tools.length === 1 ? '' : 's'}`,
      collapsible: true,
      defaultCollapsed: true,
      itemLimit: TOOL_LIST_DEFAULT_LIMIT,
      items: tools.map((tool, index) => ({
        id: `${tool.name}-${index}`,
        title: tool.name,
        subtitle: tool.type,
        text: tool.description,
      })),
    });
  }
  return {
    instanceId: action.id,
    kind: 'llm.request',
    heading: 'LLM Request',
    chips: compactChips([
      chip('model', attrs['llm.request.model'] ?? body?.model),
      chip('provider', attrs['llm.request.provider_id'] ?? body?.provider),
      chip('messages', messages.length || null),
      chip('tools', tools.length || null),
      chip('blocks', attrs['llm.request.block_count']),
      chip('bytes', attrs['llm.request.canonical_body_bytes'] ?? attrs['llm.request.payload_bytes']),
    ]),
    blocks,
  };
}

function responseInsight(action) {
  const attrs = action.attributes ?? {};
  const toolCalls = responseToolCalls(attrs['llm.response.tool_calls_json']);
  const reasoningText = String(attrs['llm.response.reasoning_text'] ?? '').trim();
  const contentText = String(attrs['llm.response.content_text'] ?? '').trim();
  const blocks = [];
  if (toolCalls.length > 0) {
    blocks.push({
      id: 'tool-calls',
      tone: 'tools',
      label: 'Tool calls',
      title: `${toolCalls.length} proposed call${toolCalls.length === 1 ? '' : 's'}`,
      items: toolCalls.map((call, index) => ({
        id: call.id ?? `${call.name}-${index}`,
        title: `${call.name} #${index + 1}`,
        subtitle: call.type,
        text: call.argumentsText,
      })),
    });
  }
  if (reasoningText) {
    blocks.push({
      id: 'reasoning',
      tone: 'reasoning',
      label: 'Reasoning',
      title: 'model reasoning',
      text: reasoningText,
    });
  }
  if (contentText) {
    blocks.push({
      id: 'content',
      tone: 'response',
      label: 'Content',
      title: 'assistant message',
      text: contentText,
    });
  }
  return {
    instanceId: action.id,
    kind: 'llm.response',
    heading: 'LLM Response',
    chips: compactChips([
      chip('model', attrs['llm.response.model']),
      chip('provider', attrs['llm.response.provider_id']),
      chip('tool calls', toolCalls.length || null),
      chip('output', attrs['llm.response.completion_tokens']),
      chip('reasoning', attrs['llm.response.reasoning_tokens']),
      chip('total', attrs['llm.response.total_tokens']),
      chip('stream', attrs['llm.response.stream']),
      chip('done', attrs['llm.response.done']),
    ]),
    blocks,
  };
}

function callInsight(action) {
  const attrs = action.attributes ?? {};
  return {
    instanceId: action.id,
    kind: 'llm.call',
    heading: 'LLM Call',
    chips: compactChips([
      chip('model', attrs['llm.call.model']),
      chip('http', attrs['http.response.status_code']),
      chip('operation', attrs['payload.operation_id']),
      chip('stream', attrs['payload.stream_key']),
    ]),
    blocks: [
      {
        id: 'links',
        tone: 'context',
        label: 'Linked actions',
        title: 'request / response',
        rows: compactRows({
          request_action_id: attrs['llm.call.request_action_id'],
          response_action_id: attrs['llm.call.response_action_id'],
          http_response_action_id: attrs['llm.call.http_response_action_id'],
        }),
      },
    ],
  };
}

function requestBodyFromContent(action, requestContent) {
  if (requestContent?.action_id && action?.id && requestContent.action_id !== action.id) {
    return null;
  }
  const raw = requestContent?.body_json;
  if (!raw) {
    return null;
  }
  const parsed = parseJson(raw);
  return parsed && typeof parsed === 'object' ? parsed : null;
}

function requestMessages(body) {
  if (!body || typeof body !== 'object') {
    return EMPTY_ARRAY;
  }
  const messages = [];
  if (body.system) {
    messages.push(normalizeMessage({ role: 'system', content: body.system }, messages.length));
  }
  if (Array.isArray(body.messages)) {
    messages.push(...body.messages.map((message, index) => normalizeMessage(message, messages.length + index)));
  } else if (Array.isArray(body.input)) {
    messages.push(...body.input.map((message, index) => normalizeMessage(message, messages.length + index)));
  } else if (typeof body.input === 'string') {
    messages.push(normalizeMessage({ role: 'input', content: body.input }, messages.length));
  } else if (typeof body.prompt === 'string') {
    messages.push(normalizeMessage({ role: 'prompt', content: body.prompt }, messages.length));
  }
  return messages.filter((message) => message.text || message.role);
}

function normalizeMessage(message, index) {
  const role = String(message?.role ?? message?.type ?? `message ${index + 1}`);
  return {
    index,
    role,
    text: requestMessageText(message),
  };
}

function requestTools(body) {
  if (!body || typeof body !== 'object') {
    return EMPTY_ARRAY;
  }
  if (Array.isArray(body.tools)) {
    return body.tools.map((tool) => normalizeToolDefinition(tool, 'tools')).filter(Boolean);
  }
  if (Array.isArray(body.functions)) {
    return body.functions.map((tool) => normalizeToolDefinition(tool, 'functions')).filter(Boolean);
  }
  return EMPTY_ARRAY;
}

function normalizeToolDefinition(tool, source) {
  if (!plainObject(tool)) {
    return null;
  }
  const fn = plainObject(tool.function) ? tool.function : tool;
  if (!validToolDefinition(tool, fn, source)) {
    return null;
  }
  const name = String(fn.name ?? tool.name ?? '').trim();
  if (!name) {
    return null;
  }
  return {
    name,
    type: toolType(tool, fn),
    description: compactToolDescription(fn.description ?? tool.description),
  };
}

function validToolDefinition(tool, fn, source) {
  if (source === 'functions') {
    return typeof fn.name === 'string' && (plainObject(fn.parameters) || typeof fn.description === 'string');
  }
  if (tool.type === 'function') {
    return plainObject(tool.function) && typeof fn.name === 'string';
  }
  if (typeof tool.name !== 'string') {
    return false;
  }
  return plainObject(tool.input_schema) || typeof tool.description === 'string';
}

function toolType(tool, fn) {
  if (tool.type) {
    return String(tool.type);
  }
  if (fn.type) {
    return String(fn.type);
  }
  return 'function';
}

function compactToolDescription(description) {
  const text = String(description ?? '')
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean);
  return previewText(text, 140);
}

function plainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function responseToolCalls(raw) {
  const parsed = typeof raw === 'string' ? parseJson(raw) : raw;
  const calls = Array.isArray(parsed) ? parsed : EMPTY_ARRAY;
  return calls.map(normalizeToolCall).filter((call) => call.name);
}

function normalizeToolCall(call, index) {
  const fn = call?.function ?? call;
  const args = firstPresent(
    fn?.arguments_json,
    parseMaybeJson(fn?.arguments),
    call?.arguments_json,
    parseMaybeJson(call?.arguments),
    call?.input,
  );
  return {
    id: call?.id,
    name: String(fn?.name ?? call?.name ?? call?.tool_name ?? 'tool').trim(),
    type: String(call?.type ?? fn?.type ?? 'function'),
    argumentsText: formatToolArguments(args, index),
  };
}

function formatToolArguments(args, index) {
  if (args === undefined || args === null || args === '') {
    return `call ${index + 1}`;
  }
  if (typeof args === 'string') {
    return args.trim();
  }
  return JSON.stringify(args, null, 2);
}

function parseMaybeJson(value) {
  if (typeof value !== 'string') {
    return value;
  }
  const parsed = parseJson(value);
  return parsed === null ? value : parsed;
}

function messagesTextByRoles(messages, roles) {
  if (!Array.isArray(messages)) {
    return '';
  }
  const allowed = new Set(roles);
  return messages
    .map((message) => formatMessageLine(message, allowed))
    .filter(Boolean)
    .join('\n');
}

function formatMessageLine(message, allowedRoles = null) {
  if (!message || typeof message !== 'object') {
    return '';
  }
  const role = String(message.role ?? '').toLowerCase();
  if (allowedRoles && role && !allowedRoles.has(role)) {
    return '';
  }
  const prefix = message.role ? `[${message.role}] ` : '';
  const content = requestMessageText(message);
  if (!content) {
    return '';
  }
  return `${prefix}${content}`.trim();
}

function requestMessageText(message) {
  if (!message || typeof message !== 'object') {
    return messageContentText(message);
  }
  return [
    messageContentText(message.content ?? message.text ?? message.input ?? message.prompt),
    reasoningContentText(message),
    messageToolCallsText(message.tool_calls ?? message.toolCalls),
  ]
    .filter(Boolean)
    .join('\n\n')
    .trim();
}

function reasoningContentText(message) {
  const value = firstPresent(message.reasoning_content, message.reasoning, message.thinking);
  const text = messageContentText(value);
  return text ? `[reasoning]\n${text}` : '';
}

function messageToolCallsText(toolCalls) {
  if (!Array.isArray(toolCalls)) {
    return '';
  }
  return toolCalls
    .map(formatMessageToolCall)
    .filter(Boolean)
    .join('\n\n');
}

function formatMessageToolCall(call, index) {
  if (!call || typeof call !== 'object') {
    return '';
  }
  const normalized = normalizeToolCall(call, index);
  if (!normalized.name) {
    return '';
  }
  const lines = [`[tool_call] ${normalized.name} #${index + 1}`];
  if (normalized.id) {
    lines.push(`id: ${normalized.id}`);
  }
  if (normalized.type) {
    lines.push(`type: ${normalized.type}`);
  }
  if (normalized.argumentsText) {
    lines.push(`arguments:\n${normalized.argumentsText}`);
  }
  return lines.join('\n');
}

function messageContentText(content) {
  if (typeof content === 'string') {
    return content.trim();
  }
  if (Array.isArray(content)) {
    return content
      .map(messageContentPartText)
      .filter(Boolean)
      .join('\n\n')
      .trim();
  }
  if (content && typeof content === 'object') {
    return messageContentPartText(content).trim();
  }
  return '';
}

function messageContentPartText(part) {
  if (typeof part === 'string') {
    return part.trim();
  }
  if (!part || typeof part !== 'object') {
    return '';
  }
  if (typeof part.text === 'string' && part.text.trim()) {
    return part.text.trim();
  }
  if (part.type === 'tool_use') {
    return formatToolUseContentBlock(part);
  }
  if (part.type === 'tool_result') {
    return formatToolResultContentBlock(part);
  }
  if (typeof part.content === 'string' && part.content.trim()) {
    return part.content.trim();
  }
  if (typeof part.input === 'string' && part.input.trim()) {
    return part.input.trim();
  }
  if (part.type === 'image_url' || part.type === 'input_image') {
    return formatMediaContentBlock(part, 'image');
  }
  if (part.type === 'document' || part.type === 'input_file') {
    return formatMediaContentBlock(part, 'document');
  }
  return stringifyContentBlock(part);
}

function formatToolUseContentBlock(part) {
  const lines = [`[tool_use] ${String(part.name ?? part.type ?? 'tool').trim()}`];
  if (part.id) {
    lines.push(`id: ${part.id}`);
  }
  if (part.input !== undefined) {
    lines.push(`input:\n${stringifyContentValue(part.input)}`);
  }
  return lines.join('\n');
}

function formatToolResultContentBlock(part) {
  const lines = [`[tool_result] ${String(part.tool_use_id ?? part.id ?? 'tool').trim()}`];
  if (part.is_error !== undefined) {
    lines.push(`is_error: ${Boolean(part.is_error)}`);
  }
  if (part.content !== undefined) {
    lines.push(stringifyContentValue(part.content));
  }
  return lines.join('\n');
}

function formatMediaContentBlock(part, label) {
  const source = part.url ?? part.image_url?.url ?? part.source?.url ?? part.file_id ?? part.media_type ?? part.type;
  return `[${label}] ${String(source ?? '').trim() || stringifyContentBlock(part)}`;
}

function stringifyContentBlock(part) {
  const serialized = stringifyContentValue(part);
  return serialized === '{}' ? '' : serialized;
}

function stringifyContentValue(value) {
  if (typeof value === 'string') {
    return value.trim();
  }
  if (value === undefined || value === null) {
    return '';
  }
  return JSON.stringify(value, null, 2);
}

function openAiChoicesText(parsed) {
  const choices = parsed?.choices;
  if (!Array.isArray(choices)) {
    return '';
  }
  return choices
    .map((choice) => {
      const message = choice?.message ?? choice?.delta;
      if (!message) {
        return choice?.text ?? '';
      }
      return messageContentText(message.content ?? message.text) || String(message.content ?? message.text ?? '').trim();
    })
    .filter(Boolean)
    .join('\n');
}

function openAiResponsesOutputText(parsed) {
  const output = parsed?.output;
  if (!Array.isArray(output)) {
    return '';
  }
  return output
    .flatMap((item) => {
      if (typeof item?.content === 'string') {
        return [item.content];
      }
      if (Array.isArray(item?.content)) {
        return item.content
          .map((part) => (typeof part?.text === 'string' ? part.text : ''))
          .filter(Boolean);
      }
      return [];
    })
    .join('');
}

function anthropicResponseText(parsed) {
  if (Array.isArray(parsed?.content)) {
    const text = parsed.content
      .map((block) => (typeof block?.text === 'string' ? block.text : ''))
      .filter(Boolean)
      .join('');
    if (text) {
      return text;
    }
  }
  const message = parsed?.message ?? parsed?.delta;
  if (message) {
    return messageContentText(message.content ?? message.text);
  }
  return '';
}

function messagesText(messages) {
  if (Array.isArray(messages)) {
    return messages
      .map((message) => formatMessageLine(message))
      .filter(Boolean)
      .join('\n');
  }
  if (typeof messages === 'string') {
    return messages.trim();
  }
  return '';
}

function parseJson(value) {
  if (typeof value !== 'string') {
    return value ?? null;
  }
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

function looksLikeJson(text) {
  return text.startsWith('{') || text.startsWith('[');
}

function messageTitle(message) {
  const role = String(message.role ?? '').trim();
  return role ? `${role} #${message.index + 1}` : `message #${message.index + 1}`;
}
