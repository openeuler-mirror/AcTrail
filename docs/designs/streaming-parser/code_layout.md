# Streaming parser code layout

## 目标目录

```text
crates/core/semantic_action_runtime/src/llm_pipeline/
├── mod.rs 最小 crate 内入口与 re-export
├── config.rs pipeline 子配置；包含 16 KiB soft sniff 配置
│
├── facade/ 唯一编排入口，不拥有协议解析细节
│   ├── mod.rs 仅声明与 re-export
│   ├── event.rs PipelineEvent → PipelineAdvance/ActionBatch 唯一数据与生命周期入口
│   ├── input.rs 构造与 codec 扩展入口
│   ├── output.rs ActionBatch
│   ├── payload.rs payload-segment ingress
│   ├── websocket.rs WebSocket observation、synthetic stream ownership 与精确清理
│   ├── pipeline.rs 生命周期与顶层编排
│
├── transport/ 只负责 wire normalization、framing 与 evidence
│   ├── mod.rs 仅声明与 re-export
│   ├── message.rs provider-neutral normalized HTTP message DTO
│   ├── buffer/ amortized O(1) prefix release
│   │   ├── mod.rs
│   │   └── cursor.rs bounded CursorBuffer
│   ├── evidence/ wire offset → payload evidence
│   │   ├── mod.rs
│   │   └── tracker.rs 单调 append/evict、range seek 与 per-response EvidenceCursor/Snapshot
│   ├── http1/ HTTP/1.x 增量解码
│   │   ├── mod.rs
│   │   ├── decoder.rs header/fixed/chunked/trailer/EOF 状态机与 fail-local decode failure
│   │   └── resynchronizer.rs 单调 request-line boundary recovery
│   ├── http2/ HTTP/2 frame 增量解码
│   │   ├── mod.rs
│   │   ├── decoder.rs frame 增量解码、跨 segment evidence、RST_STREAM 生命周期
│   │   ├── framing.rs NeedMore/Invalid/Frame、extension frame 跳过、padding 与 stream-id validation
│   └── websocket/ WebSocket handshake/framing/connection adapter
│       ├── mod.rs
│       ├── handshake.rs
│       ├── framing.rs
│       ├── connection.rs
│       └── adapter.rs
│
├── assembly/ 有状态连接组装与跨层协调
│   ├── mod.rs
│   ├── plain/
│   │   ├── mod.rs
│   │   └── aggregate.rs HTTP/1/raw logical stream aggregate
│   ├── http2/
│   │   ├── mod.rs
│   │   └── connection.rs HTTP/2 logical stream coordination
│   └── router/
│       ├── mod.rs
│       ├── identity.rs
│       ├── limits.rs
│       └── router.rs connection identity、限额与协议选择
│
├── stream/ decoded body 的分类、framing 与终止
│   ├── mod.rs 仅声明与 re-export
│   ├── classifier/ soft-window LLM candidate classifier
│   │   ├── mod.rs
│   │   └── classifier.rs
│   ├── sse_framer/ complete-event incremental SSE framing
│   │   ├── mod.rs
│   │   └── framer.rs
│   ├── response/ provider-neutral response decoding
│   │   ├── mod.rs
│   │   └── body.rs 增量 SSE cache；非 SSE JSON 仅在终结时 batch parse
│   └── finalizer/ 统一 abnormal partial 终止原因；正常完成由 projection status 决定
│       ├── mod.rs
│       └── finalizer.rs
│
├── provider/ provider detection、固定 parser driver 与 codec 扩展
│   ├── mod.rs 仅声明与 re-export
│   ├── registry/ provider/parser 选择
│   │   ├── mod.rs
│   │   ├── registry.rs
│   │   ├── request.rs
│   │   └── generic_request.rs
│   ├── driver/ confirmed stream 的 parser 驱动与聚合工具
│   │   ├── mod.rs
│   │   └── driver.rs
│   ├── codec/ 外部 codec adapter
│   │   ├── mod.rs
│   │   └── adapter.rs
│   ├── anthropic/
│   ├── openai_chat/
│   ├── openai_responses/
│   └── structured_json/
│
└── projection/ normalized provider state → exported actions
    ├── mod.rs 仅声明与 re-export
    ├── batch.rs ProjectionBatch 内部增量；facade/output.rs 导出 ActionBatch 名称
    ├── orchestrator/ facade-facing correlation/projection 协调组件
    │   ├── mod.rs
    │   ├── orchestrator.rs 状态所有权
    │   ├── admission.rs per-trace correlation stream 准入、淘汰与诊断
    │   ├── bindings.rs active/damaged response binding 生命周期
    │   ├── correlation.rs request/response/exchange/call 编排
    │   ├── http.rs HTTP request/response correlation 编排
    │   ├── lifecycle.rs trace/stream 生命周期清理
    │   ├── pending.rs 有界 pending request/response 状态与淘汰语义
    │   └── projection.rs action/trajectory/version 编排
    ├── projector/ request/response/action projection owner
    │   ├── mod.rs
    │   ├── projector.rs trajectory、deferred action、version 去重
    │   ├── state.rs per-trace 有界状态的有序索引与容量淘汰
    │   ├── request.rs request action、content 与 payload projection
    │   ├── request/
    │   │   └── tool_results.rs provider-neutral tool-result projection
    │   ├── response.rs
    │   ├── live.rs decoded HTTP/1、HTTP/2 与 raw stream projection adapter
    │   ├── http.rs HTTP failure/partial action projection
    │   └── support.rs
    ├── correlation/ request/response/exchange/call state owner
    │   ├── mod.rs
    │   ├── ownership.rs per-trace binding/stream ownership 有界索引
    │   ├── coordinator.rs correlation state
    │   └── call.rs
    ├── links/ HTTP request/response link proposals
    │   ├── mod.rs
    │   └── proposals.rs
    ├── retention/ evidence、payload draft、request blocks
    │   ├── mod.rs
    │   ├── evidence.rs
    │   ├── policy.rs
    │   ├── request_blocks.rs
    │   └── request_blocks/
    │       ├── canonical_json.rs
    │       └── metadata.rs
    └── trajectory/ bounded trajectory classifier
        ├── mod.rs
        └── classifier/
            ├── mod.rs
            ├── implementation.rs classifier state 与 bounded inference
            └── implementation/
                ├── prefix.rs bounded prefix trie
                └── provider.rs provider response correlation
```

## 目标依赖方向

```text
L0 capture (external)
    │ PayloadSegment / stream close / gap / HTTP exchange
    ▼
facade
    ├──► assembly ────► transport ───► evidence
    ├──► stream ──────► provider
    └──► projection ──► retention / trajectory / links
                         │
                         └──► ActionBatch (external export boundary)
```

允许的依赖：

```text
facade      -> transport, stream, provider, projection
facade      -> assembly
assembly    -> transport, stream, provider, projection
transport   -> transport::evidence
stream      -> provider
provider    -> provider::codec
projection  -> retention, trajectory, links, provider normalized types
```

禁止的依赖：

```text
transport   -X-> provider
transport   -X-> projection
transport   -X-> stream
provider    -X-> transport
projection  -X-> facade
stream      -X-> facade
```

## 目标边界类型

```text
PipelineEvent facade 的统一输入
DecodedHttp1Message HTTP/1 normalized message snapshot，共享累积 body
Http2DecodeBatch HTTP/2 DATA/end/reset/protocol-failure 增量批次
ProviderStreamUpdate provider-neutral 增量响应状态
ProjectionBatch 投影动作、provider identity、关联更新
ActionBatch 唯一外部导出批次
```
