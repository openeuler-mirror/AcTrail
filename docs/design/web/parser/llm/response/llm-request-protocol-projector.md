# LLM Request Protocol Projector 规范

## 背景

AcTrail 从 HTTP、TLS plaintext 和 WebSocket 中恢复 LLM request。不同客户端和 provider 使用不同 JSON shape；即使都使用 OpenAI 风格字段，message、tool 和 input item 的位置与语义也不相同。

当前至少存在三种输入。

Chat Completions 风格：

```json
{
  "model": "example-model",
  "messages": [
    { "role": "developer", "content": "..." },
    { "role": "user", "content": "..." }
  ],
  "tools": [
    { "type": "function", "name": "..." }
  ]
}
```

Responses 风格：

```json
{
  "model": "example-model",
  "input": [
    {
      "type": "message",
      "role": "developer",
      "content": [
        { "type": "input_text", "text": "..." }
      ]
    }
  ],
  "tools": []
}
```

Codex Responses Lite 风格：

```json
{
  "type": "response.create",
  "model": "example-model",
  "input": [
    {
      "type": "additional_tools",
      "role": "developer",
      "tools": []
    },
    {
      "type": "message",
      "role": "developer",
      "content": [
        { "type": "input_text", "text": "..." }
      ]
    }
  ]
}
```

第三种 shape 中，`input` 是异构 item 序列；`role = developer` 不表示该 item 一定是文本 message。若 Web 把所有 `input[]` 都映射成 message，就会把 `additional_tools` 显示为空白 developer block，并遗漏其中的 tool definitions。

本文定义一个 request protocol projector 框架：上游负责把 transport bytes 组装成完整 JSON；projector 负责识别 dialect，并将原始 shape 投影成统一、有序、可解释的 request item 模型。Web 只消费统一模型，不再识别 `messages`、`input`、`additional_tools` 或 provider 私有字段。

## 1. 文档地位

本文是 LLM request shape 识别、归一化、存储和 Web 消费的目标规范。

本文使用以下规范词：

- **必须**：实现不得违反。
- **禁止**：实现不得采用。
- **应该**：除非有明确且可记录的工程理由，否则应遵循。
- **可以**：允许采用，但不是强制要求。

## 2. 目标

Request projector 必须满足：

1. 对完整 JSON 执行无状态 dialect 匹配和统一投影。
2. 显式区分 message、tool set、prompt 和未知 item。
3. 保留原始 item 顺序、角色、来源路径和 dialect identity。
4. Chat Completions、Responses 与 Codex Responses Lite 通过相同 contract 参与选择。
5. 多个同等级 projector 命中时返回歧义，不静默选择注册顺序中的第一个。
6. 未知 item 不得被丢弃或伪装成空 message。
7. canonical raw content 与 normalized projection 同时存在，各自承担不同职责。
8. Web 不得重新实现 provider/dialect JSON 解析。
9. 新增 dialect 或 item type 以追加 projector/decoder 为主，不破坏已验证 dialect。

## 3. 非目标

本框架不负责：

- TLS plaintext probe、socket capture 或 payload retention policy。
- WebSocket frame、permessage-deflate、HTTP body 或 SSE event 的字节组装。
- 模型 response body、stream delta、token usage 或 tool call output 的解析。
- 用 normalized projection 替代 canonical raw request 的审计和精确重建。
- 强制合法 request 同时包含 message 和 tools。
- 在前端维护 provider-specific fallback parser。

Transport assembler 与 request protocol projector 必须分离：

```text
TLS/socket/WebSocket bytes
  → transport assembler
    → complete request JSON
      → request protocol projector
```

只有 transport assembler 是增量状态机。对完整 `serde_json::Value` 工作的 protocol projector 必须是无状态对象。

## 4. 统一 contract

推荐 contract：

```rust
trait LlmRequestProtocolProjector: Send + Sync {
    fn projector_id(&self) -> &'static str;

    fn match_request(
        &self,
        context: &LlmRequestProjectionContext<'_>,
    ) -> LlmRequestMatch;

    fn project(
        &self,
        context: &LlmRequestProjectionContext<'_>,
    ) -> Result<NormalizedLlmRequest, LlmRequestProjectionError>;
}
```

`match_request` 必须是纯函数，不得改变 projector 内部状态。`project` 只能读取 context，不得修改全局 parser registry 或 retention state。

不采用下列 stateful request-shape API：

```text
on_input_data(bytes)
hit()
done()
reset()
```

原因：

1. request shape projector 的输入已经是完整 JSON。
2. `hit()` 应由当前输入决定，而不是由上一次请求残留状态决定。
3. `done()` 不能定义为“同时存在 messages 和 tools”；无 tools、prompt-only、tool-only extension 都可能合法。
4. bytes/frame 生命周期属于 transport assembler，不能与 JSON dialect 生命周期混合。

若未来需要增量 JSON parser，应定义独立的 assembler contract，输出完整 JSON 后再调用本文 projector。

## 5. Projection Context

一次投影共享的只读事实应集中在 context：

```rust
struct LlmRequestProjectionContext<'a> {
    json: &'a serde_json::Value,
    transport: LlmRequestTransport,
    protocol_hint: Option<&'a str>,
    route: Option<&'a str>,
    source_boundary: PayloadSourceBoundary,
}
```

`protocol_hint` 是证据，不是强制选择结果。Projector 必须验证实际 JSON shape；禁止仅凭 WebSocket path、HTTP route、provider 名称或客户端进程名宣称强匹配。

## 6. 匹配和选择

匹配必须至少区分：

```rust
enum LlmRequestMatch {
    NoMatch,
    Plausible(LlmRequestMatchEvidence),
    Strong(LlmRequestMatchEvidence),
}
```

推荐 projector 顺序不是选择优先级；选择由 match strength 和显式策略决定。

目标 registry：

```text
CodexResponsesLiteRequestProjector
ResponsesRequestProjector
ChatCompletionsRequestProjector
GenericJsonRequestProjector
```

最低匹配要求：

- Codex Responses Lite：`type = response.create`，且 `input` 含该 dialect 支持的异构 item；匹配为 `Strong`。
- Responses：`input` 满足 Responses item shape；顶层 `tools` 可选；匹配为 `Strong` 或 `Plausible`。
- Chat Completions：`messages` 是 message array；顶层 `tools` 可选；匹配为 `Strong`。
- Generic JSON：具有 `model`，并含 `messages`、`input` 或 `prompt` 之一；只能返回 `Plausible`。

选择结果必须显式表达：

```rust
enum LlmRequestProjectionOutcome {
    Unsupported(LlmRequestProjectionEvidence),
    Matched(NormalizedLlmRequest),
    Ambiguous(AmbiguousLlmRequestProjection),
}
```

规则：

1. 选择唯一最高 match strength 的 projector。
2. 同一最高 strength 有多个互不等价 projector 时返回 `Ambiguous`。
3. `NoMatch` candidate 可以从本轮候选集中移除，但不得删除 raw request。
4. 所有 projector 都 `NoMatch` 时返回 `Unsupported`，并保留 canonical raw content。
5. `ProjectionError` 表示已匹配 projector 无法兑现其 contract，不能降级成另一个宽松 projector 来掩盖损坏输入。

## 7. 统一 Request 模型

统一模型必须以有序 item 为主，不得只返回两个彼此独立的 `MessageBlocks` 和 `ToolList`：

```rust
struct NormalizedLlmRequest {
    projection_version: u32,
    projector_id: String,
    protocol_id: String,
    model: Option<String>,
    items: Vec<LlmRequestItem>,
    warnings: Vec<LlmRequestProjectionWarning>,
}

enum LlmRequestItem {
    Message(LlmRequestMessage),
    ToolSet(LlmRequestToolSet),
    Prompt(LlmRequestPrompt),
    Unknown(LlmRequestUnknownItem),
}
```

保持 `items` 顺序是必须条件。格式 3 可以在 message 前后插入 `additional_tools`；若只生成独立 messages/tools 数组，会丢失原始相对顺序和来源位置。

### 7.1 Message

```rust
struct LlmRequestMessage {
    role: Option<String>,
    name: Option<String>,
    content: Vec<LlmRequestContentPart>,
    source: LlmRequestItemSource,
}

enum LlmRequestContentPart {
    Text { text: String },
    Image { url: Option<String>, detail: Option<String> },
    ToolResult { call_id: Option<String>, content: String },
    Refusal { text: String },
    Unknown { type_name: Option<String> },
}
```

禁止因为 item 有 `role` 就把它直接认定为 message。必须验证 item type 和 message content shape。

### 7.2 Tool Set

```rust
struct LlmRequestToolSet {
    role: Option<String>,
    tools: Vec<LlmRequestToolDefinition>,
    source: LlmRequestItemSource,
}

struct LlmRequestToolDefinition {
    name: Option<String>,
    kind: String,
    description: Option<String>,
    input_schema: Option<serde_json::Value>,
    children: Vec<LlmRequestToolDefinition>,
}
```

`children` 用于 namespace tool。Function、custom、namespace 和 tool_search 等类型必须保留真实 kind；禁止全部重写为 function。

### 7.3 Source

每个 item 必须记录来源：

```rust
enum LlmRequestItemSource {
    Messages { index: usize },
    TopLevelTools { index: usize },
    Input { index: usize },
    AdditionalTools { input_index: usize },
    Prompt,
}
```

Source 用于诊断、UI 标签和未来重新投影，不应作为展示层重新解析 raw JSON 的入口。

### 7.4 Unknown Item

未知 item 必须保留：

```rust
struct LlmRequestUnknownItem {
    type_name: Option<String>,
    source: LlmRequestItemSource,
    reason: String,
}
```

Normalized model 可以不复制未知 item 的完整敏感 payload，但必须保留类型、位置和拒绝原因；完整内容由 canonical raw blocks 保证。

## 8. Dialect Projector

### 8.1 Chat Completions

`ChatCompletionsRequestProjector`：

- `messages[]` 投影为有序 `Message`。
- 顶层 `tools[]` 或兼容 `functions[]` 投影为 `ToolSet`。
- string content 和 content-part array 使用共享 message content decoder。
- 未知 message content part 生成 `Unknown` part，不删除整个 message。

### 8.2 Responses

`ResponsesRequestProjector`：

- 遍历 `input[]`，按 item type 调用共享 item decoder。
- 顶层 `tools[]` 投影为独立 `ToolSet`。
- string `input` 投影为 `Prompt`。
- 不识别的 input item 生成 `Unknown`。

### 8.3 Codex Responses Lite

`CodexResponsesLiteRequestProjector`：

- 验证 `type = response.create` 和 Codex Responses Lite shape evidence。
- 复用 Responses message/content decoder。
- 将 `input[].type = additional_tools` 投影为 `ToolSet`，不得投影为空 developer message。
- 支持 function、custom、namespace、tool_search 及 namespace 内嵌套 tools。
- 保留 `role`，但 `role` 只描述来源语义，不改变 item kind。

### 8.4 Generic JSON

Generic projector 只提供安全的最低限度投影：

- 提取 model。
- 对可证明是 message 的 item 生成 `Message`。
- 无法证明的 shape 生成 `Unknown`。
- 禁止用递归搜索任意 `text`/`tools` key 的方式伪造结构。

## 9. 共享 Decoder

格式 2 和格式 3 都属于 Responses input-item family，必须共享 item decoder，而不是复制完整 projector。

```text
ResponsesInputItemDecoder
├── MessageItemDecoder
├── AdditionalToolsItemDecoder
├── PromptItemDecoder
├── ToolResultItemDecoder
└── UnknownItemDecoder
```

Codex projector 负责 envelope identity 和 dialect-specific enablement；共享 decoder 负责 item shape。只有明确属于 Codex 的扩展语义进入 Codex namespace。

## 10. Canonical Raw 与 Normalized Projection

两类数据必须并存：

```text
canonical raw request
  → 精确重建
  → 内容审计
  → hash/dedup
  → 新 projector 版本重新解析

normalized request projection
  → Web Message context
  → Available tools
  → 搜索、统计和跨 provider 展示
```

Normalized projection 禁止替代 canonical request content。Projector 升级后，同一 raw body 可能产生更完整的 normalized projection；因此必须记录 `projection_version` 和 `projector_id`。

内容 retention 仍由现有 semantic retention policy 控制。Projector 不得通过 attributes、日志或 diagnostics 复制本应只存在于 canonical blocks 的完整 prompt/tool description。

## 11. API 与 Web 边界

Request content API 应返回：

```json
{
  "action_id": "...",
  "projection": {
    "version": 1,
    "projector_id": "codex-responses-lite",
    "protocol_id": "websocket.responses",
    "model": "example-model",
    "items": []
  },
  "raw": {
    "available": true,
    "truncated": false
  }
}
```

完整 `body_json` 可以由显式 raw-content API 或兼容字段提供，但 Web 默认视图必须消费 `projection.items`。

展示规则：

```text
Message      → Message context
ToolSet      → Available tools
Prompt       → Prompt
Unknown      → Unsupported request block
```

前端禁止根据 `messages`、`input`、`additional_tools`、`body.tools` 或 `body.functions` 再次推断协议。

## 12. 完整性与失败语义

Projection completeness 不等于同时存在 message 和 tools。

推荐状态：

```rust
enum LlmRequestProjectionCompleteness {
    Complete,
    Partial,
}
```

- `Complete`：所有已知 item 均成功投影，允许 messages 或 tools 为空。
- `Partial`：request dialect 已确定，但包含未知/损坏 item。
- `Unsupported`：没有 projector 命中。
- `Ambiguous`：多个同等级 projector 命中且不能证明等价。
- `Err`：已命中 projector 无法完成其承诺的结构校验。

Partial projection 必须保留 warnings；Web 可以展示已知 item，同时标记未知 block，不得把 partial 伪装成 complete。

## 13. 版本、Evidence 与诊断

每次 projection 至少记录：

- projector ID 和版本。
- protocol ID。
- match strength。
- 命中的 envelope/item evidence。
- unknown item 数量和 source path。
- selection ambiguity 的 candidate IDs。

Evidence 不得包含完整 prompt、tool description、schema 或认证字段。诊断输出只记录 shape、数量、类型和拒绝原因。

## 14. 迁移顺序

建议迁移顺序：

1. 在 semantic action contract 中加入 normalized request model。
2. 扩展现有 request registry，使其返回 projection outcome，而不只返回 classifier/model。
3. 实现 Chat Completions 和 Responses projector，并复用共享 decoder。
4. 实现 Codex Responses Lite projector 和 `additional_tools` decoder。
5. Request content API 同时提供 projection 与兼容 raw content。
6. Web Message context/Available tools 改为只消费 projection。
7. 移除前端 provider-specific `messages/input/tools/functions` 解析。
8. 保留 canonical raw storage，不做破坏性迁移。

迁移期可以保留旧 raw API，但禁止在新前端路径中把 raw parser 作为 normalized projection 失败后的静默 fallback。

## 15. 评审准则

新增或修改 projector 时必须回答：

1. 该 dialect 的强匹配证据是什么。
2. 是否会与已有 projector 产生同等级歧义。
3. 每种 input item 如何投影，未知 item 如何保留。
4. 是否保持原始 item 顺序和 source path。
5. 是否复用已有 message/tool decoder。
6. 是否避免把敏感内容复制到 attributes/diagnostics。
7. 是否同时保留 canonical raw content。
8. Web 是否完全不需要知道新增的 wire-format key。
