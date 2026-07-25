# LLM Request Protocol Projector 当前实现路径地图

本文件分解当前 request 采集、识别、canonical retention 和 Web 展示路径。当前实现已经能无损保存 Codex Responses Lite 的 `additional_tools` block，但尚未形成统一 messages/tools projection；Web 仍直接解析 raw request JSON。

```text
crates/core/semantic_action_runtime/src/
├── live/llm/
│   ├── websocket.rs
│   │   ├── 组装 WebSocket frame、fragment 和 permessage-deflate
│   │   ├── 识别 outbound response.create
│   │   └── 投影为 synthetic HTTP request payload
│   └── llm.rs
│       └── 将 synthetic payload 送入通用 HTTP/LLM projection
│
└── payload_projection/llm/
    ├── request.rs
    │   ├── LlmRequestBodyParser：读取完整 HTTP body
    │   ├── 调用 request registry 识别 request
    │   ├── 当前仅取得 classifier_id、protocol_id 和 model
    │   └── 根据 retention policy 生成 llm.request action 与 content write
    │
    ├── provider/
    │   ├── request_registry.rs
    │   │   ├── LlmRequestParser contract
    │   │   ├── NoMatch/Plausible/Strong 选择
    │   │   ├── 同等级 ambiguity 拒绝
    │   │   └── ParsedLlmRequest 当前不包含 messages/tools/items
    │   ├── generic_request.rs
    │   │   └── model + messages/prompt/input 的宽松 request 识别
    │   └── structured_json_sse.rs
    │       └── 特定结构化 JSON/SSE request 识别
    │
    └── request_blocks.rs
        ├── canonicalize 完整 request body
        ├── messages、tools、prompt 和 input 拆分为 deduplicated blocks
        ├── skeleton 保留 block ordinal placeholders
        └── 不解释 input item 是 Message 还是 ToolSet

crates/contracts/semantic_action/src/
├── llm.rs
│   └── 当前主要定义 response parser、token usage 和 tool call contract
└── model.rs
    ├── LlmRequestManifest
    ├── LlmRequestBlockRef
    ├── LlmRequestBlock
    ├── LlmRequestContentWrite
    └── LlmRequestContentPage

crates/storage/adapters/sqlite/src/semantic_actions/
└── llm_request_content/
    ├── write.rs
    │   └── 写入 manifest、block refs 和 deduplicated blocks
    └── read.rs
        └── 按 ordinal 重建 canonical body_json

crates/apps/web/src/
├── view/actions.rs
│   └── llm_request_content_json 返回重建后的 raw body_json
└── http.rs
    └── /api/traces/{trace}/actions/{action}/content/llm-request

crates/apps/web/frontend/src/
└── llm/insight.js
    ├── requestBodyFromContent：解析 raw body_json
    ├── requestMessages：自行判断 system/messages/input/prompt
    ├── requestTools：只读取顶层 tools/functions
    └── 当前把 input.additional_tools 作为空 developer message，
        且无法将其 tools 投影到 Available tools
```

当前真实 Codex shape：

```text
response.create
└── input[]
    ├── additional_tools(role=developer, tools=[...])
    └── message(role=developer, content=[input_text])
```

当前前端解释：

```text
input[0] → developer #1，因没有 content/text 而显示空白
input[1] → developer #2，显示 input_text
body.tools/body.functions → 不存在，因此 Available tools 为空
```

已具备且应保留的实现：

1. WebSocket bytes 到完整 request JSON 的组装。
2. canonical request hash、block ordinal、dedup 和精确重建。
3. request registry 的 match strength 与 ambiguity 骨架。
4. semantic retention 对 request content 的控制。

需要迁移的职责：

1. `ParsedLlmRequest` 从 metadata-only 扩展为统一 item projection。
2. `additional_tools` 等 dialect-specific item 在 Rust projector 中解释。
3. Web API 返回 normalized projection。
4. 前端移除 wire-format parser，仅渲染 Message/ToolSet/Prompt/Unknown。

当前 canonical block 被记录不等于已经被语义解析。Raw retention 的成功只能证明内容未丢失，不能证明 Web 已正确理解其中的 item kind。
