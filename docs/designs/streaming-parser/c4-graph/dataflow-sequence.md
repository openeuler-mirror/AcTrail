# Streaming parser 数据流时序

本文定义 LLM 流式响应的目标数据处理流程。配套文档包括 [容器图](container.puml)、[组件图](component.puml) 和 [分层职责](layer.md)。

核心约束是：HTTP transport 必须先归一化，SSE framer 只能消费连续的 decoded body；provider parser 只聚合语义，`SemanticAction` 必须由掌握 HTTP exchange context 的 projector 生成。

## HTTP/1.1 chunked SSE

下面的响应在一个 SSE JSON event 中间发生 TLS 分片：

```text
TLS segment 1:
HTTP/1.1 200 OK\r\n
Transfer-Encoding: chunked\r\n
Content-Type: text/event-stream\r\n
\r\n
31\r\n
data: {"choices":[{"delta":{"content":"Hel

TLS segment 2:
lo"}}]}\n\n
12\r\n
data: [DONE]\n\n
0\r\n\r\n
```

```mermaid
sequenceDiagram
    participant Hook as TLS Hook
    participant Stream as PlainStreamAssembly
    participant HTTP as Incremental HTTP/1 Decoder
    participant SSE as Incremental SSE Framer
    participant Classifier as Stream Classifier
    participant Provider as Provider Parser
    participant Projector as HTTP-aware Projector
    participant Storage as Semantic Storage

    Hook->>Stream: segment 1: headers + chunk framing + half event
    Stream->>HTTP: push transport bytes
    HTTP->>HTTP: parse headers and available chunk framing
    HTTP-->>SSE: shared decoded body snapshot: data: ... "Hel
    Note over HTTP,SSE: HTTP headers和chunk-size不得进入SSE parser
    SSE->>SSE: retain incomplete event tail
    SSE-->>Classifier: no complete event
    Classifier-->>Provider: NeedMore
    SSE-->>Stream: retain framer cursor + compact evidence snapshot
    Note over Stream: transport boundary 未完成，wire prefix 不提前释放

    Hook->>Stream: segment 2: remaining JSON + DONE + zero chunk
    Stream->>HTTP: push transport bytes
    HTTP-->>SSE: extended shared body: lo"}}]} + DONE
    SSE->>SSE: join pending tail
    SSE-->>Classifier: complete choices event
    Classifier-->>Provider: ConfirmedLlm(OpenAI-compatible)
    Provider->>Provider: aggregate content += "Hello"
    SSE-->>Provider: complete [DONE] event
    Provider->>Provider: mark completed
    SSE-->>Stream: cursor advanced through complete events
    HTTP-->>Stream: zero chunk confirms transport boundary
    Stream->>Stream: release complete encoded message

    Provider-->>Projector: complete LlmResponseAggregate
    HTTP-->>Projector: status + exchange metadata + offset mapping
    Projector->>Projector: build llm.response and paired llm.call
    Projector->>Storage: persist actions, links and retained evidence
```

第一段的半个事件只能进入 `SseFramer.pending_event`，不能参与 provider 分类。第二段补全空行终止符后，framer 才交付完整 event。

目标接口形态：

```rust
fn push_http1_transport(bytes: &[u8]) {
    assembly.append(bytes);
    if let Some(message) = http1_decoder.advance(assembly.remaining(), false)? {
        let events = sse_framer.advance(&message.body)?;
        provider.observe(events);
        if message.complete { assembly.release(message.encoded_len); }
    }
}
```

## HTTP/2 SSE

HTTP/2 同时存在 frame 跨 TLS callback 和 SSE event 跨 DATA frame 两种边界。connection decoder 必须先拼出完整 frame，再按 `stream_id` 解复用。

```mermaid
sequenceDiagram
    participant Hook as TLS Hook
    participant Conn as HTTP/2 Connection Decoder
    participant Demux as Stream Demultiplexer
    participant SSE7 as SSE Framer stream=7
    participant Classifier7 as Classifier stream=7
    participant Provider7 as Provider Parser stream=7
    participant Projector as HTTP-aware Projector
    participant Storage as Semantic Storage

    Hook->>Conn: TLS segment 1: HEADERS + partial DATA frame
    Conn->>Conn: decode complete HEADERS(stream=7)
    Conn->>Demux: response context(stream=7)
    Conn->>Conn: retain incomplete frame tail
    Note over Conn: partial frame不能退化为unknown payload

    Hook->>Conn: TLS segment 2: rest of DATA + next DATA
    Conn->>Conn: complete frame decoding
    Conn->>Demux: DATA(stream=7): data: ... "Hel
    Demux->>SSE7: push decoded DATA payload
    SSE7-->>Classifier7: no complete event
    Classifier7-->>Provider7: NeedMore

    Conn->>Demux: DATA(stream=7): lo"}} + blank line
    Demux->>SSE7: push decoded DATA payload
    SSE7-->>Classifier7: complete Anthropic event
    Classifier7-->>Provider7: ConfirmedLlm(Anthropic)
    Provider7->>Provider7: aggregate content += "Hello"

    Conn->>Demux: DATA(stream=7, END_STREAM): message_stop
    Demux->>SSE7: advance shared body snapshot to final cursor
    SSE7-->>Provider7: complete message_stop event
    Provider7->>Provider7: mark completed

    Provider7-->>Projector: complete aggregate
    Demux-->>Projector: stream_id=7
    Conn-->>Projector: HTTP/2 response context
    Projector->>Projector: build response with stream_id=7
    Projector->>Projector: match request on stream_id=7
    Projector->>Storage: persist response, call and links
    SSE7-->>Demux: framer cursor advanced
    Demux->>Demux: remove stream after terminal projection/END_STREAM
    Conn->>Conn: release processed frame bytes
```

每个 HTTP/2 stream 必须拥有独立 pipeline：

```rust
struct Http2StreamPipeline {
    stream_id: u32,
    response_context: Http2ResponseContext,
    sse: IncrementalSseFramer,
    classification: StreamClassification,
    provider: Option<Box<dyn ProviderStreamParser>>,
    aggregate: LlmResponseAggregate,
}
```

`stream_id` 由 transport/projector 层持有，不能依赖 SSE parser 重建。

## LLM SSE soft-window 分类

首个完整 event 可能只是 Anthropic `ping`，因此一次未识别不能进入永久 fallback。16 KiB 是初始识别 soft window，不是数据保留或丢弃阈值。

```mermaid
sequenceDiagram
    participant SSE as SSE Framer
    participant Classifier as Bounded Classifier
    participant LLM as Provider Parser
    participant Buffer as Stream Buffer

    SSE->>Classifier: complete ping event
    Classifier-->>SSE: Undetermined
    Note over Classifier: ping不能证明该流不是LLM
    SSE->>Classifier: complete message_start event
    Classifier-->>LLM: ConfirmedLlm(Anthropic)
    SSE->>LLM: content_block_delta
    LLM->>LLM: aggregate semantic content
    SSE-->>Buffer: release completed framing bytes

    alt 超过 soft window 后才出现决定性 event
        SSE->>Classifier: later complete provider event
        Classifier-->>LLM: late ConfirmedLlm
        Note over Classifier,Buffer: 不因 soft budget 丢弃、清空或永久降级
    end
```

建议状态模型：

```rust
enum StreamClassification { Undetermined, ConfirmedLlm }
```

`Undetermined` 的 raw assembly 仍受 `max_buffer_bytes` 与 `max_segment_ranges` 硬限制。达到硬限制时只 fail-local 当前 stream；不得让故障传播到 payload ingestion、其他 HTTP/2 stream 或 daemon。

## 关闭、截断与超限

DONE、HTTP/2 `END_STREAM`、trace close、capture gap、payload truncation 和配置上限都必须进入同一个 finalizer。

```mermaid
sequenceDiagram
    participant Transport as HTTP Transport
    participant Pipeline as SSE Stream Pipeline
    participant Provider as Provider Parser
    participant Projector as HTTP-aware Projector
    participant Storage as Semantic Storage

    Transport--xPipeline: unexpected close / gap / truncation / limit
    Pipeline->>Provider: finish incomplete stream

    alt ConfirmedLlm且已有有效语义内容
        Provider-->>Projector: meaningful partial aggregate
        Projector->>Projector: build Error/Partial response and call
        Projector->>Storage: persist partial actions and compact diagnostic
    else 未确认或没有有效语义内容
        Pipeline->>Storage: persist compact diagnostic
    end

    Pipeline->>Pipeline: release only this stream state
    Note over Transport,Storage: fail-local，不影响其他stream或上游采集
```

## 每次推进后的资源不变量

每次 `push` 返回前必须满足：

1. transport decoder 使用 cursor，已完成 frame 会及时释放；HTTP/1 wire message 在边界确认后整体释放。
2. SSE framer 只扫描累计 body 的新增后缀，不重复复制或全量解析既有 event。
3. provider parser 只保留聚合语义与协议状态，不保留所有 raw events。
4. 只有证明完整的 encoded message/frame prefix 才从 assembly buffer 释放。
5. evidence 使用 compact span mapping；是否保留原 payload 由 retention 配置决定。
6. 任一 stream 的 malformed/limit/close 只终止该 stream，并尽可能生成 partial action。
