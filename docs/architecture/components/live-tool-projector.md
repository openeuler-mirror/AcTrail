# Live Tool Projector

> 本文展示工具调用、工具结果和 Agent 调用如何在实时语义运行时中关联为可查询的 action 与 link。

Live Tool Projector 是 `LiveSemanticActionRuntime` 内部的有状态组件。它消费已经标准化的大语言模型（LLM）action、工具结果和请求 lineage，生成 `llm.tool_call`、`llm.tool_result`、`agent.invocation` action，以及它们之间的语义 link。**Action** 是一次有明确类型和时间范围的语义活动记录；**link** 表示两条 action 的关系；**lineage** 指请求与父请求、前序响应之间的来源关系。

## 边界与输入输出

`LiveSemanticActionRuntime` 是唯一调用者。它通过 `ToolProjectionBatch` 一次提交以下输入：

- LLM pipeline 已生成的 `llm.request` 和 `llm.response` action；
- 从 request body 中识别出的工具结果；
- 用于输出请求链关系的 request lineage。

投影器返回 `ToolProjectionOutput`，其中只包含新增 action、link 和 LLM pipeline 诊断。运行时将这些结果与同批其他语义结果合并，再交给统一持久化路径。

该组件不解析 TLS、HTTP、服务器发送事件（Server-Sent Events，SSE）或模型服务方的传输格式，也不负责存储和导出。输入在到达这里之前已经完成传输组装与 LLM 协议投影。

## 组件职责

| 组件 | 当前职责 |
|---|---|
| `ToolInteractionProjector` | 按批次顺序编排声明解码、状态更新、关联和输出 |
| `DeclaredLlmToolCalls` | 从 `llm.response` attributes 解码声明的 tool call，并报告损坏或缺失名称的条目 |
| `AgentInvocationCorrelator` | 按工具结果 ID 关联结果，并按 Agent 工具策略识别调用 |
| `ToolInteractionState` | 独占主记录、查询索引、trace ownership 和容量淘汰 |
| `ToolSemanticEmitter` | 构造 action、link 和诊断，并持有本批输出 |

状态和关联分离：Correlator 只能通过状态对象的语义方法查询候选，不能直接修改内部 map；所有主记录和辅助索引的变更由同一个状态方法完成。

## 一批数据如何投影

1. 投影器先遍历 `llm.response`，解码其中声明的 tool call。
2. 每个有效声明生成 `llm.tool_call`；符合 Agent 工具策略的声明同时建立或更新 `agent.invocation`。
3. 投影器从本批 `llm.request` 建立查找表，再将每个工具结果绑定到对应 tool call。缺少 ID、没有候选或候选不唯一时，结果仍可形成 action，同时产生生命周期诊断。
4. Emitter 一次返回完整输出，运行时将其与 LLM pipeline 的 action、content 和 lineage 合并。

实时运行时不比较委派 prompt 与子请求，不保存匹配索引或用于匹配的完整用户文本副本，也不生成 `agent.invocation.child_llm_request`。工具调用和结果仍通过 tool call ID 关联，Agent 调用生命周期仍完整记录。`llm.request.message_preview` 只保留有界展示文本。

## Web 离线委派关联

Web 进入 LLM Trajectory 页面时，以持久化的调用记录和请求原文为唯一依据，按需计算 `delegation` 虚线边。边连接父 request 与子 trajectory 的首个 request，携带 invocation ID，结果只属于当前视图，不写回数据库。

`OfflineAgentCorrelation` 通过调用记录中的 tool call action ID、response action ID、ordinal，以及同一 `llm.call` 的 `llm.call.request` / `llm.call.response` 两条观测关系定位委派 prompt，不从 action ID 字符串推断身份。候选请求须完整、非后台、属于另一条 trajectory，且开始于调用开始之后；已有 append/fork 来源的历史节点不参与委派入口匹配。最新有效用户消息的完整文本块及双换行组合参与精确匹配；调用和请求两侧均唯一时才生成边。多个匹配、缺失或截断内容不产生猜测关系。

工具结果可能只是异步子任务的启动回执，因此 invocation 的结束时间不能作为子请求开始的上界。

请求原文由现有 canonical content API 读取；`shape` 或 `none` 保留模式没有完整原文时，页面保留既有轨迹并提示委派关系可能不完整。切换 trace 或离开页面会取消读取并释放当前匹配状态。

`LlmTrajectoryTab.correlationOptions` 可配置正整数 `concurrency`（默认 4）和 `maxBytes`（默认 8388608，每个请求的读取上限）。同一配置传入 `OfflineAgentCorrelation` 构造器，非法值立即报错；内容读取故障仅影响当前视图的推断。

## Trace 生命周期与故障边界

每个 trace 的记录数受启动配置限制。容量耗尽时，状态对象按既定顺序淘汰条目，Emitter 产生数据丢弃诊断；解析或关联失败不会使实时运行时退出。

`finish_trace` 会先物化仍有意义的未完成 Agent 调用，再确定性清理该 trace 的记录和索引。`forget_trace` 不产生输出，只清理该 trace。两种路径都通过 trace ownership 定点删除，不扫描全部 trace 状态。

## 源码位置

```text
crates/core/semantic_action_runtime/src/live/
├── runtime.rs                 # 唯一调用者与输出合并
└── tool/
    ├── contract.rs            # ToolProjectionBatch / ToolProjectionOutput
    ├── projector.rs           # 投影 façade
    └── internal/
        ├── declaration/       # tool call 声明解码
        ├── correlation/       # 工具结果关联与 Agent 工具识别
        ├── state/             # 记录、索引、容量和 trace 清理
        └── emission/          # action、link 与诊断构造
```
