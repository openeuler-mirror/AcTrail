# Streaming parser 分层职责

本文规定目标 streaming parser 的层级边界、所有权和性能不变量。数据时序见 [dataflow-sequence.md](dataflow-sequence.md)，结构关系见 [容器图](container.puml) 和 [组件图](component.puml)。

## 设计原则

目标流水线是：

```text
TLS/socket plaintext
→ HTTP transport normalization
→ SSE event framing
→ bounded stream classification
→ provider semantic aggregation
→ HTTP-aware action projection
→ persistence/export
```

任何层都不得解析属于上游尚未归一化的协议。尤其禁止：

```text
HTTP headers/chunk framing/H2 frames
→ raw SSE parser
```

SSE parser 不得直接生成缺少 HTTP identity 的 `SemanticAction`。

## Layer 0：Payload capture 与 ingress

职责：

- 采集进程内 TLS plaintext 或 socket plaintext。
- 保留 trace、process、direction、operation、sequence、capture completeness。
- 将 segment 有序交给 HTTP/semantic runtime。
- 独立持久化低层 payload；不依赖 LLM parser 成功。

不负责：

- HTTP message boundary。
- SSE event framing。
- provider classification。

故障规则：采集截断或 gap 必须显式进入 stream finalizer；不能伪装成完整 body。

## Layer 1：Assembly stream router

职责：

- 使用 `(trace, process, connection, direction)` 定位 connection state。
- HTTP/2 激活后，再以 `stream_id` 定位 logical response stream。
- 对不同 connection/stream 的状态和错误做隔离。

建议对象：

```rust
struct AssemblyStreamRouter {
    connections: Map<ConnectionKey, ConnectionPipeline>,
    limits: StreamLimits,
}
```

assembly 是唯一允许同时依赖 transport、stream、provider 与 projection 的协调层。这里的 map 必须有配置化上限与低开销淘汰策略。淘汰只关闭目标 stream，并触发 partial finalization。

## Layer 2：HTTP transport normalization

### HTTP/1 decoder

职责：

- 增量解析 response headers。
- 增量处理 `Content-Length` 或 chunked encoding。
- 输出 `DecodedHttp1Message`；累计 body 使用 `Arc<Vec<u8>>` 共享，增量追加不重复复制全文。
- 维护 decoded body offset 到 transport offset 的映射。
- 仅在 message transport boundary 完成后释放 wire prefix；SSE parser 通过内部 cursor 只扫描新增后缀。

状态只允许保留：

- 未完整 header。
- 未完整 chunk-size line。
- 当前 chunk 尚未收到的部分。
- pipelined message boundary 所需的最小尾部。

### HTTP/2 connection decoder

职责：

- 跨 TLS callback 拼接完整 HTTP/2 frame。
- 解析 HEADERS/DATA/END_STREAM/RST_STREAM。
- 不得把 partial frame 当作 unknown application payload。
- 按 `stream_id` 将 DATA payload交给 per-stream pipeline。

HTTP/2 frame decoder只保留一个未完整 frame所需的尾部；完整 frame必须及时释放。

实际边界模型：

```rust
struct DecodedHttp1Message {
    body: Arc<Vec<u8>>,
    encoded_len: usize,
    complete: bool,
    body_boundary_known: bool,
}

struct Http2DecodeBatch {
    data: Vec<Http2DataEvent>,
    ended: Vec<Http2EndStreamEvent>,
    failures: Vec<Http2StreamFailureEvent>,
}
```

## Layer 3：Incremental SSE framer

职责：

- 观察持续增长的共享 decoded body，并从单调 scan cursor 开始处理新增后缀。
- 支持 LF、CRLF 以及跨多次 `advance` 的行/空行边界。
- 只输出具有完整空行终止符的 SSE event。
- 聚合多行 `data:`，保留需要的 `event:` 和 `id:`。
- 保存累计 body 的已扫描 cursor；每次只处理新增且已经完整闭合的 event。

不负责：

- 判断 event 是否属于 LLM。
- 解析 provider JSON。
- 生成 action。

建议接口：

```rust
impl IncrementalSseFramer {
    fn advance(&mut self, accumulated_body: &[u8]) -> Result<Vec<CompleteSseEvent>, FramingError>;
}
```

性能不变量：每个 body byte 最多参与常数次扫描；不得在每个 delta 到达时重新扫描整个累计 body。

## Layer 4：Bounded stream classifier

职责：

- 只观察完整 `SseEvent`。
- 在 soft initial window 内优先识别 OpenAI-compatible、OpenAI Responses、Anthropic 或 codec，并允许后续完整事件触发 late recognition。
- 对 `ping`、metadata、comment 等非决定性事件保持 `Undetermined`。
- 确认 provider 后固定 parser，不在每个 event 上重新检测。

状态模型：

```rust
enum StreamClassification { Undetermined, ConfirmedLlm }
```

`UndeterminedWindow` 使用
`semantic_retention.l0_llm_call.stream_classifier.soft_sniff_max_bytes`
限制用于 provider 识别的 decoded SSE body 前缀，默认值为 16384 字节。达到
soft budget 后保持 `Undetermined` 并允许逐 event late recognition；不得据此清空 buffer、丢弃
event 或永久判定为 ordinary SSE。本 action pipeline 不拥有 ordinary SSE 消费者，因此不会建立虚假的 handoff 模块。该配置必须为正、可表示为 `usize`，且不得超过
`assembly.max_buffer_bytes`，所有约束均在启动时校验。

## Layer 5：Provider stream parser

职责：

- 为每条已确认 LLM stream 创建一个固定 parser object。
- 增量处理 OpenAI-compatible、Responses、Anthropic 或 codec-normalized event。
- 聚合 text、reasoning、tool calls、tool arguments、usage、model、finish reason 和 provider response id。
- 判断 `in_progress`、`complete` 和 `meaningful_partial`。

建议结果模型：

```rust
struct LlmResponseAggregate {
    provider: ProviderKind,
    provider_response_id: Option<String>,
    model: Option<String>,
    text: String,
    reasoning: String,
    tool_calls: Vec<ToolCall>,
    usage: Option<TokenUsage>,
    finish_reason: Option<String>,
}
```

parser拥有 aggregate，但不拥有全部 raw event 列表。tool call 数量、arguments bytes、字段数量等必须有配置上限。

## Layer 6：Evidence span tracking

职责：

- 将 decoded body range 映射回 transport/payload segment range。
- 在释放 raw assembly prefix 后仍能构建 action evidence。
- 使用单调 range tracker 与 per-response cursor/snapshot，不复制完整 payload。
- 遵循 retention 配置决定是否额外生成 assembled payload draft。

建议模型：

```rust
struct EvidenceSpan {
    body_start: u64,
    body_end: u64,
    payload_sequence_start: u64,
    payload_sequence_end: u64,
    operation_id: u64,
}
```

range 数量受 `assembly.max_segment_ranges` 硬限制；cursor 只向前扫描，snapshot 按 segment id 去重，并按 operation 独立验证连续性。

## Layer 7：HTTP-aware semantic projector

职责：

- 接收 immutable complete/partial aggregate。
- 从 HTTP runtime 获取 protocol、status、request identity、H2 `stream_id` 和 confirmed exchange。
- 从 span tracker 获取 sequence/evidence range。
- 应用 semantic retention 配置。
- 生成 `llm.response`、`llm.call`、payload draft 和 links。

必须由这一层生成 action：

```rust
fn project_response(
    aggregate: LlmResponseAggregate,
    http: HttpResponseContext,
    evidence: EvidenceRanges,
    retention: &SemanticRetentionConfig,
) -> ProjectionOutput;
```

SSE/provider 层不得自行拼装 action，否则容易丢失 HTTP/2 `stream_id`、provider response identity、request link 或 retention/evidence 语义。

## Layer 8：Stream finalization 与 diagnostics

所有异常 partial 终止原因进入统一 finalizer；正常 success 由 provider progress 与 HTTP transport completion 共同决定：

```rust
enum StreamFinishReason {
    TraceClose,
    CaptureGap,
    PayloadTruncated,
    BufferLimitExceeded,
    ProtocolMismatch,
    StreamReset,
}
```

规则：

- provider DONE/finish reason 与完整 HTTP boundary 生成 terminal success response。
- meaningful partial aggregate生成 `Error/Partial` response和call。
- 未确认或无有效语义内容只生成 compact diagnostic。
- diagnostics不得包含重 payload、headers或token正文。
- finalization只清理当前 logical stream；下游写失败不能异常传播到其他stream或上游采集。

## Ordinary SSE handoff

确认普通 SSE 后：

1. 将有界窗口内保留的完整 events 移交 ordinary SSE/application protocol路径。
2. 释放 provider候选状态和不再需要的 LLM evidence。
3. 后续 event不再进入 provider registry。
4. completed framing bytes持续释放，避免长连接占满 LLM assembly。

ordinary SSE path尚未消费事件时，也必须受独立配置上限约束，不能以 fallback 名义无限缓存。

## 配置边界

至少需要以下分层配置；具体字段加入现有配置前必须单独评审：

```text
streaming_parser.transport.max_connections
streaming_parser.transport.max_http2_streams_per_connection
streaming_parser.http.max_header_bytes
streaming_parser.http.max_frame_tail_bytes
streaming_parser.sse.max_pending_event_bytes
semantic_retention.l0_llm_call.stream_classifier.soft_sniff_max_bytes
streaming_parser.provider.max_aggregate_bytes
streaming_parser.provider.max_tool_calls
streaming_parser.evidence.max_spans
```

启动时配置缺失、为零或相互矛盾必须 fail-fast。运行时达到限制必须 fail-local，并带 compact reason code。

## 性能与正确性验收标准

实现完成至少满足：

- transport、SSE、provider每层对新增字节执行摊销 O(n) 工作，不重复解析完整累计响应。
- 长寿命 ordinary SSE 的 LLM assembly内存保持有界且不随事件总数增长。
- LLM SSE只保留一个不完整 event、语义 aggregate和有界 evidence spans。
- HTTP/1 chunk boundary、TLS segment boundary、SSE event boundary三者任意错位仍能解析。
- HTTP/2 frame跨 callback、event跨 DATA frame、多个stream交错时仍保持 per-stream identity。
- `ping` 或 metadata首事件不会导致永久 ordinary/LLM误判。
- malformed provider event只影响当前stream；已有有效内容在关闭时生成partial。
- action保留provider response id、HTTP/2 stream id、sequence范围和request/response links。
- retention关闭正文时，不因解析实现额外持久化正文。

端到端验证必须使用刷新后的默认配置运行真实 agent，并至少覆盖 Pi/xiaoO、HTTP/1 chunked、HTTP/2、多事件长流、跨边界分片和异常关闭。

## 与旧 SSE assembly 设计的关系

本设计继承旧方案的早分类、固定parser、move ownership、低内存和partial目标，但修正两个边界：

1. SSE framing只能发生在 HTTP transport normalization之后。
2. action generation只能发生在掌握HTTP exchange context的projector层。

旧 `docs/designs/llm-sse-assembly.md` 中“raw input直接进入SSE framing”和“新parser直接生成SemanticAction”的描述不适用于本目标架构。
