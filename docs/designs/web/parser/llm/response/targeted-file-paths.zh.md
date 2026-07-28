# LLM Request Protocol Projector 目标文件路径

本文件定义 LLM request protocol projector 重构完成后的目标路径。路径本身承担 namespace：contract 定义跨 runtime、storage、Web API 和 frontend 共享的统一语义；registry 只负责 match/selection；每个 dialect projector 负责 envelope 识别和 item 投影；Responses family 共享 item decoder；canonical retention 与 normalized projection 保持并行。

目录名中的 `response` 指 Responses family 协议 namespace。`response.create` 是 request envelope；模型返回内容仍由既有 LLM response parser 处理，不进入本文 request projector tree。

```text
crates/contracts/semantic_action/src/
├── llm/
│   ├── mod.rs
│   │   └── 最小 re-export request projection 与既有 response parsing contracts
│   │
│   └── request/
│       ├── mod.rs
│       │   └── 仅声明 request contract namespace 并执行最小 re-export
│       │
│       ├── projection/
│       │   ├── mod.rs
│       │   └── normalized_llm_request.rs
│       │       └── NormalizedLlmRequest
│       │           ├── projection_version
│       │           ├── projector_id/protocol_id/model
│       │           ├── ordered items
│       │           └── warnings
│       │
│       ├── item/
│       │   ├── mod.rs
│       │   ├── llm_request_item.rs
│       │   │   └── Message、ToolSet、Prompt、Unknown
│       │   └── source.rs
│       │       └── Messages、TopLevelTools、Input、AdditionalTools、Prompt source path
│       │
│       ├── message/
│       │   ├── mod.rs
│       │   ├── message.rs
│       │   │   └── role、name、ordered content parts 与 source
│       │   └── content_part.rs
│       │       └── Text、Image、ToolResult、Refusal、Unknown
│       │
│       ├── tool/
│       │   ├── mod.rs
│       │   ├── tool_set.rs
│       │   │   └── role、ordered tool definitions 与 source
│       │   └── tool_definition.rs
│       │       └── function/custom/namespace/tool_search identity、schema 与 children
│       │
│       └── outcome/
│           ├── mod.rs
│           ├── completeness.rs
│           │   └── Complete/Partial；不得以 messages+tools 是否同时存在判断
│           └── warning.rs
│               └── unknown item、unsupported content part 和保留原因
│
└── lib.rs
    └── 暴露稳定 request projection contract

crates/core/semantic_action_runtime/src/payload_projection/llm/
├── mod.rs
│   └── 声明 request、response、codec 与 live projection namespace
│
└── request/
    ├── mod.rs
    │   ├── 取代当前单文件 request.rs
    │   ├── 组合 body parsing、protocol projection、retention 和 action projection
    │   └── 不包含 dialect-specific key 判断
    │
    ├── body.rs
    │   └── 从已组装 HTTP/synthetic HTTP body 解析完整 JSON
    │
    ├── action_projector.rs
    │   └── 将 projection metadata 写入 llm.request action；
    │       完整敏感 message/tool content 不复制到 attributes
    │
    ├── retention/
    │   ├── mod.rs
    │   └── canonical_blocks.rs
    │       ├── 迁入现有 request_blocks.rs
    │       ├── 保持 canonical hash、skeleton、block refs 和 dedup
    │       └── normalized projection 失败时仍可保存 raw content
    │
    └── protocol_projector/
        ├── mod.rs
        │   └── 仅声明 contract、registry、selection、shared decoder 与 dialect tree
        │
        ├── contract/
        │   ├── mod.rs
        │   ├── context.rs
        │   │   └── JSON、transport、protocol hint、route、source boundary 只读事实
        │   ├── projector.rs
        │   │   └── LlmRequestProtocolProjector
        │   │       ├── projector_id
        │   │       ├── match_request(context) -> LlmRequestMatch
        │   │       └── project(context) -> NormalizedLlmRequest
        │   ├── match.rs
        │   │   └── NoMatch、Plausible、Strong 与 shape evidence
        │   ├── outcome.rs
        │   │   └── Unsupported、Matched、Ambiguous
        │   ├── evidence.rs
        │   │   └── projector path、shape keys、item counts 与拒绝原因；禁止敏感正文
        │   └── error.rs
        │       └── 已匹配 projector 无法兑现结构 contract 的错误
        │
        ├── registry.rs
        │   ├── 持有全部 dialect projectors
        │   ├── 选择唯一最高 match strength
        │   └── 同等级非等价命中返回 Ambiguous
        │
        ├── selection.rs
        │   └── 统一 selection 逻辑；禁止依赖注册顺序静默选第一个
        │
        ├── shared/
        │   ├── mod.rs
        │   ├── message_content_decoder.rs
        │   │   └── string/content array 的 Text、Image、ToolResult、Refusal、Unknown
        │   └── tool_definition_decoder.rs
        │       └── function/custom/namespace/tool_search 与递归 namespace children
        │
        └── dialect/
            ├── mod.rs
            │   └── 声明具体 dialect projector namespace
            │
            ├── chat_completions/
            │   ├── mod.rs
            │   └── projector.rs
            │       ├── messages[] → Message items
            │       ├── top-level tools[] → ToolSet
            │       └── functions[] compatibility → ToolSet
            │
            ├── responses/
            │   ├── mod.rs
            │   ├── projector.rs
            │   │   ├── input[] → ordered items
            │   │   ├── string input → Prompt
            │   │   └── top-level tools[] → ToolSet
            │   │
            │   └── input_item/
            │       ├── mod.rs
            │       ├── decoder.rs
            │       │   └── 按 item type 分派，并为 Codex extension 提供显式 enablement
            │       ├── message.rs
            │       │   └── type=message → Message
            │       ├── prompt.rs
            │       │   └── prompt/input string → Prompt
            │       ├── tool_result.rs
            │       │   └── 已知 tool result item → MessageContentPart::ToolResult
            │       ├── additional_tools.rs
            │       │   └── type=additional_tools → ToolSet；不得生成空 Message
            │       └── unknown.rs
            │           └── 未知 input item → Unknown + warning
            │
            ├── codex_responses_lite/
            │   ├── mod.rs
            │   └── projector.rs
            │       ├── 验证 type=response.create 和 Codex Responses Lite evidence
            │       ├── 复用 responses/input_item decoder
            │       ├── 启用 additional_tools 和 Codex tool kinds
            │       └── 保留 role 但不以 role 推断 item kind
            │
            └── generic_json/
                ├── mod.rs
                └── projector.rs
                    ├── 仅返回 Plausible
                    ├── 只投影可证明的 message/prompt
                    └── 无法证明的结构生成 Unknown，不递归猜测任意 text/tools key

crates/storage/adapters/sqlite/src/semantic_actions/
├── llm_request_content/
│   ├── write.rs
│   │   └── 继续存储 canonical manifest、refs 与 blocks
│   └── read.rs
│       └── 继续提供精确 raw body 重建
│
└── llm_request_projection/
    ├── mod.rs
    ├── write.rs
    │   └── 按 projection_version 存储 normalized metadata/items；
    │       不绕过 semantic retention 复制完整敏感正文
    └── read.rs
        └── 读取版本化 projection；需要时允许从 canonical raw 显式重投影

crates/apps/web/src/
├── view/llm/
│   ├── mod.rs
│   └── request_projection.rs
│       └── 输出 normalized projection API，不判断 wire-format keys
│
├── view/actions.rs
│   └── raw canonical content 保留为显式兼容/审计路径
│
└── http.rs
    ├── request projection endpoint
    └── raw request content endpoint；两者职责不得混合成前端静默 fallback

crates/apps/web/frontend/src/llm/
├── insight.js
│   ├── Message → Message context
│   ├── ToolSet → Available tools
│   ├── Prompt → Prompt
│   └── Unknown → Unsupported request block
└── insight.test.js
    └── 只验证统一 projection 的展示，不构造 Chat/Responses/Codex wire JSON
```

目标数据流：

```text
HTTP/WebSocket assembler
  → complete JSON
    ├── canonical raw retention
    └── LlmRequestProtocolProjector registry
        ├── ChatCompletionsRequestProjector
        ├── ResponsesRequestProjector
        ├── CodexResponsesLiteRequestProjector
        └── GenericJsonRequestProjector
          → NormalizedLlmRequest
            → versioned storage/API
              → protocol-agnostic Web rendering
```

目标 namespace 示例：

```text
semantic_action::llm::request::projection
semantic_action::llm::request::item
semantic_action::llm::request::message
semantic_action::llm::request::tool

semantic_action_runtime::payload_projection::llm::request::protocol_projector::contract
semantic_action_runtime::payload_projection::llm::request::protocol_projector::dialect::chat_completions
semantic_action_runtime::payload_projection::llm::request::protocol_projector::dialect::responses
semantic_action_runtime::payload_projection::llm::request::protocol_projector::dialect::codex_responses_lite
```

`ResponsesRequestProjector` 与 `CodexResponsesLiteRequestProjector` 必须复用 `responses/input_item/`。只有 Codex envelope evidence、Codex extension enablement 和 Codex-specific tool kinds 进入 `codex_responses_lite/`；message/content/tool 的通用结构不得复制。

Canonical raw 与 normalized projection 禁止合并成单一存储 contract：

```text
canonical raw
├── body hash
├── exact skeleton
├── ordered block refs
└── deduplicated encoded blocks

normalized projection
├── projector/protocol/version
├── ordered semantic items
├── source paths
├── completeness
└── warnings/evidence
```

迁移完成必须同时满足：

1. Rust registry 返回统一 projection，而不只返回 classifier/model。
2. Codex `additional_tools` 被投影为 ToolSet，不再成为空 developer message。
3. Web frontend 不再读取 `messages`、`input`、`additional_tools`、`tools` 或 `functions` 判断协议。
4. Unknown item 可见且可诊断，不静默丢弃。
5. canonical raw request 仍可精确重建。
6. projector ambiguity、partial 和 unsupported 具有独立结果，禁止用空 projection 混淆。
