// Request-view defaults; LlmRequestBlocks accepts readConcurrency and entriesPerPage overrides.
export const REQUEST_CONTENT_LIMITS = Object.freeze({ readConcurrency: 3, fieldsPerPage: 50, entriesPerPage: 5, summaryCharacters: 600, compactValueCharacters: 160 });
export const PROSE_FIELDS = Object.freeze(['text', 'description', 'content', 'input', 'output', 'arguments', 'prompt', 'system', 'instructions']);
