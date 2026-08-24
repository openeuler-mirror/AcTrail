# LLM SSE Assembly Refactor

Status: in progress (step 1 done)

## Problem

Current LLM SSE projection buffers/parses all SSE streams as if they might be
LLM. Ordinary SSE and non-LLM REST JSON should be classified as early as
possible, so they do not hold LLM assembly state or memory.

## Goals

- Early classify SSE stream as LLM or fallback.
- Use an enum state to select parser; do not re-detect on every event.
- Only the LLM parser owns a cross-event buffer.
- On fallback, transfer buffer ownership by move; no copy.
- On protocol mismatch or incomplete end: emit diagnostic and fallback.
- Preserve ordinary SSE / REST JSON path and low-level body retention.
- Keep memory and CPU overhead low.

## Data model

```text
HTTP
├── JSON
│   ├── LLM JSON
│   └── ordinary REST JSON
└── SSE
    ├── LLM SSE
    │   ├── openai-chat
    │   ├── openai-responses
    │   └── anthropic
    └── ordinary SSE
```

Processing chain:

```text
raw input
├── direct JSON
│     → parse JSON
│     → classify LLM / non-LLM
└── SSE
      → SSE framing
      → event.data
      → classify LLM / non-LLM
```

SSE raw framing (`data:`, blank lines, chunked bytes) is a transport detail and
must not be retained after framing.

## State machine

```text
Default
  → only first complete event does candidate detection
  → non-LLM → Fallback
  → LLM candidate → Llm(parser)

Llm(parser)
  → subsequent events go directly to selected parser
  → parser owns buffer
  → protocol mismatch → take_buffer() -> Fallback
  → complete+verified → create action and clean

Fallback
  → no cross-event buffer
  → each event handled immediately
  → may receive owned buffer from failed LLM parser
```

## Ownership rules

- LLM parser owns `Vec<EventData>`.
- `try_acc` pushes, never copies old events.
- On fallback, `take_buffer()` moves ownership to fallback parser.
- After `take_buffer`, LLM parser no longer manages that memory.
- Fallback parser decides how to process/release the moved buffer.

## Action generation ownership

The new SSE assembly state machine is responsible for early classification,
fallback, memory ownership, and action generation. The LLM parser extracts the
real LLM payload from SSE event.data, assembles it, and when verified produces
the `SemanticAction` directly from the assembled result. The old projection path
is only used for compatibility paths that have not yet migrated to the new
parser.

## Parser responsibilities

Each LLM parser:

- extracts the real LLM payload from SSE event.data (one more layer)
- maintains assembly state:
  - text deltas
  - tool_call index aggregation
  - function.arguments fragments
  - finish/termination signal
- distinguishes:
  - `enclosed()`: syntactic completeness
  - `verified()`: semantic confirmation and can produce action
- generates `llm.response` / `llm.call` / tool_calls and cleans up

## Termination signals

| protocol | signal |
| --- | --- |
| OpenAI Chat | `data: [DONE]` / `finish_reason` |
| OpenAI Responses | `response.completed` |
| Anthropic | `message_stop` |
| ordinary SSE | stream/chunked end |

If an LLM candidate ends without verified completion:

- emit diagnostic
- clean LLM parser state
- move buffer to fallback parser

## Storage / diagnostics

- Do not put heavy content into action metadata.
- Use dedicated structured table/columns for flow/diagnostic data.
- Use compact code enums, not long self-describing strings.
- Raw payload/heavy content belongs in payloads table, not diagnostics.

## TODO

- [x] Step 1: EventData + SseParserKind + SseAssemblyState skeleton
- [x] Step 2: detect_candidate for openai-chat / responses / anthropic / generic
- [x] Step 3: LlmSseParser try_acc / enclosed / verified / take_buffer
- [x] Step 4: FallbackSseParser try_acc / process(owned buffer)
- [x] Step 5: Unit tests for ordinary SSE, LLM text SSE, tool-call SSE, fallback, incomplete end
- [x] Step 6: Integrate with PlainStreamAssembly / project_inbound_responses
- [x] Step 7: Wire fallback to ordinary SSE output path (LLM layer drops non-LLM raw SSE; application protocol handles ordinary SSE)
- [x] Step 8: Add diagnostic for incomplete LLM candidate (currently tracing warn; structured table remains future work)
- [x] Step 9: Validate memory ownership (no clone on fallback)
- [x] Step 10: Run full build and targeted tests
