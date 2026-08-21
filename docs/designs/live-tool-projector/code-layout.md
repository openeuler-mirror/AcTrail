# Live Tool Projector code layout

## 模块定位

Live Tool Projector 是 `LiveSemanticActionRuntime` 内部的有状态组件。它消费 LLM pipeline 已经标准化的 `llm.request`、`llm.response`、tool result 和必要的 lineage context，跨批次关联并生成：

- `llm.tool_call`、`llm.tool_result` 和 `agent.invocation` actions；
- tool call、tool result、agent invocation 和 child request 之间的语义 links；
- 解码失败、生命周期缺口和容量耗尽等 diagnosis events。

它不负责 TLS/socket、HTTP、SSE 或 provider codec 解析，不负责持久化和导出，也不决定全局 trajectory。模块在系统中的位置见 [container.puml](c4-graphs/container.puml)，内部组件关系见 [component.puml](c4-graphs/component.puml)。

## 外部能力约定

`live/tool/` 顶层只向 `live` 模块暴露 façade 和边界 DTO。唯一调用者是 `LiveSemanticActionRuntime`。

```rust
pub(in crate::live) struct ToolInteractionProjector {
    // 所有实现状态均为 private
}

impl ToolInteractionProjector {
    pub(in crate::live) fn new(
        config: AgentInvocationConfig,
        max_entries_per_trace: u32,
    ) -> Self;

    pub(in crate::live) fn project(
        &mut self,
        batch: ToolProjectionBatch<'_>,
    ) -> ToolProjectionOutput;

    pub(in crate::live) fn finish_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> ToolProjectionOutput;

    pub(in crate::live) fn forget_trace(&mut self, trace_id: TraceId);
}
```

三个运行期能力具有不同语义：

- `project` 消费一个内部一致的标准化批次，增量生成 actions、links 和 diagnosis events；
- `finish_trace` 物化仍有意义的未完成状态，然后确定性释放该 trace 的全部内部资源；
- `forget_trace` 不产生输出，仅确定性释放该 trace 的全部内部资源。

输入应封装为借用型 DTO，避免调用者分别传递多个平铺 slice 时遗漏字段或打乱批次关系：

```rust
pub(in crate::live) struct ToolProjectionBatch<'a> {
    pub(in crate::live) actions: &'a [SemanticAction],
    pub(in crate::live) tool_results: &'a [ProjectedLlmToolResult],
    pub(in crate::live) request_lineages: &'a [LlmRequestLineageWrite],
}

#[derive(Default)]
pub(in crate::live) struct ToolProjectionOutput {
    pub(in crate::live) actions: Vec<SemanticAction>,
    pub(in crate::live) links: Vec<SemanticActionLink>,
    pub(in crate::live) diagnostics: Vec<LlmPipelineDiagnostic>,
}
```

当前只有一个进程内调用者，不引入 source/sink trait、动态分发、消息总线或同步锁。若未来出现多个宿主实现，应在 `live` runtime 边界定义端口，而不是让 tool projector 依赖外部 transport。

## 目标目录

```text
crates/core/semantic_action_runtime/src/live/tool/
├── mod.rs                         # 只声明模块并最小 re-export façade/contract
├── projector.rs                   # 唯一 façade；只做流程编排
├── contract.rs                    # ToolProjectionBatch / ToolProjectionOutput
│
└── internal/                      # tool 模块外不可见的全部实现
    ├── mod.rs                     # 只声明内部组件并做最小 re-export
    │
    ├── declaration/               # Declared Call Interpreter
    │   ├── mod.rs
    │   └── declared_calls.rs      # 值对象、arguments 解码、decode report
    │
    ├── state/                     # ToolInteractionState 聚合根
    │   ├── mod.rs
    │   ├── state.rs               # 原子 record/admit/bind/finalize/forget
    │   ├── records.rs             # ToolCallRecord / AgentInvocationRecord
    │   └── indexes.rs             # lookup、trace ownership、eviction order
    │
    ├── correlation/               # Agent Invocation Correlator
    │   ├── mod.rs
    │   ├── correlator.rs          # call-result 与 invocation-child 候选判定
    │   └── prompt_fingerprint.rs  # PromptFingerprint 值对象
    │
    └── emission/                  # Tool Semantic Emitter
        ├── mod.rs
        ├── emitter.rs             # 持有本批 ToolProjectionOutput
        ├── actions.rs             # 私有 action/diagnostic 构造细节
        └── links.rs               # 私有 tool-specific link 构造细节
```

顶层目录只显示模块边界。阅读者进入 `live/tool/` 时，首先看到唯一入口 `projector.rs` 和唯一数据约定 `contract.rs`，无需先理解 records、indexes、fingerprint 或 action attributes。

每个内部 C4 component 对应一个次级目录。`mod.rs` 不承载业务逻辑，只做模块声明和必要的最小 re-export。叶子目录均少于 10 个文件，单文件保持低于 700 行，超过 500 行时优先按内部职责继续拆分。

## 组件职责

### `ToolInteractionProjector`

Projector 是 application façade，只负责：

1. 按批次顺序调用 declaration、state、correlation 和 emission；
2. 区分增量投影、trace finish 和 trace forget；
3. 保证一次调用只返回一个完整的 `ToolProjectionOutput`。

Projector 不直接持有 `BTreeMap`，不直接修改 lookup index，也不拼装 action attributes。

### `DeclaredLlmToolCalls`

`DeclaredLlmToolCalls::from_response` 是解码值对象入口。它负责从标准化 `llm.response` attributes 中读取 tool calls、解析 string/object arguments，并返回 calls 和 decode report。

该组件是无状态解释器，不为了形式再增加空的 `Decoder` struct。

### `ToolInteractionState`

State 是最重要的一致性边界，唯一拥有：

- tool-call、tool-result 和 agent-invocation 主记录；
- tool-call ID、prompt 和 trace ownership lookup；
- admission position 和 eviction order；
- per-trace capacity；
- finalize、forget 和 eviction cleanup。

它通过语义方法暴露行为，例如：

```rust
impl ToolInteractionState {
    fn record_tool_call(...);
    fn record_tool_result(...);
    fn record_agent_invocation(...);
    fn tool_call_candidates(...);
    fn agent_child_candidates(...);
    fn complete_invocation(...);
    fn finish_trace(...);
    fn forget_trace(...);
}
```

主记录和索引的变更必须在一个 state 方法内完成。Projector 和 correlator 都不能取得裸 map 的可变引用。

### `AgentInvocationCorrelator`

Correlator 负责 agent tool policy、prompt fingerprint 和候选选择规则。它通过 `ToolInteractionState` 的只读语义查询取得候选，不直接访问或维护 state 的 map。

`PromptFingerprint` 将 hashes 和 preview 封装为不可分离的值对象：

```rust
struct PromptFingerprint {
    message_hashes: BTreeSet<String>,
    preview: Option<String>,
}
```

State 保存关联所需的中性 key 数据，不能反向调用 correlator，从而保持单向依赖。

### `ToolSemanticEmitter`

Emitter 持有当前调用的 `ToolProjectionOutput`，通过行为方法累积 actions、links 和 diagnosis events：

```rust
struct ToolSemanticEmitter {
    output: ToolProjectionOutput,
}

impl ToolSemanticEmitter {
    fn emit_tool_call(...);
    fn emit_tool_result(...);
    fn emit_agent_invocation(...);
    fn emit_link(...);
    fn diagnose(...);
    fn finish(self) -> ToolProjectionOutput;
}
```

`actions.rs` 和 `links.rs` 只是 emitter 的私有构造细节。这样既避免 projector 中平铺大量构造函数，也避免创建没有状态和不变量的空 Factory。

## 依赖方向

```text
live/runtime.rs
    │
    ▼
tool::{ToolInteractionProjector, ToolProjectionBatch, ToolProjectionOutput}
    │
    ├──► internal/declaration
    ├──► internal/correlation ───► internal/state 的只读语义查询
    ├──► internal/state
    └──► internal/emission ──────► contract

internal/* ───► model_core / semantic_action / llm_pipeline normalized DTO
```

禁止的依赖包括：

```text
internal/*       -X-> live/runtime
internal/*       -X-> storage/export
state            -X-> correlator
emission         -X-> state 的内部 records/indexes
declaration      -X-> state/correlation/emission
tool projector   -X-> TLS/HTTP/SSE/provider transport
```

通用 LLM trajectory parent/fork link 最终应由 trajectory/link 组件生成。Tool projector 只消费判断 child request 所需的最小 lineage context。迁移该职责会改变当前模块边界，应作为独立改动处理，不能夹带在目录搬迁中。

## 状态与性能不变量

1. 每个主记录都属于一个 trace ownership index。
2. 每个 lookup entry 都能定点回到仍存在的主记录。
3. 删除主记录时，同一操作必须同步删除 lookup 和 eviction order。
4. 每个 trace 的状态数量受启动时验证的配置上限约束。
5. `finish_trace` 和 `forget_trace` 后，该 trace 不得残留主记录或辅助索引。
6. trace 清理开销为 `O(k log n)`，其中 `k` 是该 trace 的记录数；禁止扫描全局状态。
7. 新布局不引入全量 canonicalization、额外 SHA256、同步锁或无界缓存。
8. 运行中解析、匹配或容量故障必须 fail-local；需要丢弃数据时产生 diagnosis event，不能 panic 或静默丢弃。

## 现有文件迁移映射

| 当前文件 | 目标位置 | 目标职责 |
|---|---|---|
| `projector.rs` | `projector.rs` | 仅保留 façade 与流程编排 |
| `projector.rs::ToolInteractionOutput` | `contract.rs` | 模块输出 DTO，并新增借用型输入 DTO |
| `declared_calls.rs` | `internal/declaration/declared_calls.rs` | 保持值对象式解码 |
| `records.rs` | `internal/state/records.rs` | 仅由 `ToolInteractionState` 使用 |
| `indexes.rs` | `internal/state/indexes.rs` | 仅由 `ToolInteractionState` 操作 |
| `prompt_fingerprint.rs` | `internal/correlation/prompt_fingerprint.rs` | 封装成 `PromptFingerprint` |
| `actions.rs` | `internal/emission/actions.rs` | 作为 emitter 私有构造细节 |
| `links.rs` | `internal/emission/links.rs` | 仅构造 tool-specific links |

迁移应先建立目标目录和类型，再逐项接线。第一阶段只改变布局与所有权封装，保持 action IDs、attributes、link roles、diagnosis codes、匹配规则、容量语义和 trace 生命周期行为不变。任何通用 lineage 职责迁移、匹配算法或淘汰语义调整都必须单独设计和验证。
