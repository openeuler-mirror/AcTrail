# LLM Trajectory 关系图设计

状态：设计草案

关联需求：[AtomGit Issue #34](https://atomgit.com/openeuler/AcTrail/issues/34)
适用范围：`actrailweb`、semantic action storage、LLM request projection

## 1. 背景与目标

同一个 Agent 会连续发出多次 LLM request。请求历史之间可能是：

- 严格前缀延续：新请求在旧请求历史后继续追加内容；
- 严格前缀分叉：一个历史节点产生多个后续分支，常见于 subagent 或并行执行；
- 完全重复：请求历史与已有请求相同；
- 上下文改写或压缩：发生 compaction 后不再满足严格前缀，但仍与此前 trajectory 相关；
- 无关的新会话。

本需求新增一个类似 Git History 的 Trace 级视图，按时间展示 LLM request 节点、trajectory 延续和分叉关系，并提供前缀匹配、重复节点等统计信息。

第一阶段只展示当前分类器已经确认的强关系，即 `append` 和 `fork_root`。内容相关但不是完整前缀的虚线关系不在第一阶段推断，避免 Web 层根据展示需要生成不可靠关系。

## 2. 当前仓库能力盘点

当前代码已经完成了大部分底层建模：

| 能力 | 当前实现 | 结论 |
| --- | --- | --- |
| trajectory 分类 | `crates/core/semantic_action_runtime/src/llm_pipeline/projection/trajectory/classifier/implementation.rs` | 已有严格前缀 Trie、append、fork、duplicate 分类 |
| lineage 领域模型 | `crates/contracts/semantic_action/src/model.rs` | 已有 `LlmRequestLineage`、transition、start reason |
| lineage 持久化 | `crates/storage/adapters/sqlite/src/semantic_actions/llm_request_lineage.rs` | 已有父节点、分叉源、位置和推断版本 |
| lineage 表与索引 | `crates/storage/adapters/sqlite/src/schema.rs` | 已有 `llm_request_lineage`，MVP 不需迁移表结构 |
| 单节点 lineage API | `GET /api/traces/{trace_id}/actions/{action_id}/lineage/llm-request` | 已有 |
| 单 trajectory API | `GET /api/traces/{trace_id}/llm-trajectories/{trajectory_id}` | 已有，但只返回 lineage，不包含节点展示信息，也不能一次返回整张图 |
| request 展示属性 | `llm.request.model`、`block_count`、`user_message_count` 等 | 大部分已具备 |
| response/tool 关系 | semantic action links | 已有 `llm.call.response`、`llm.response.tool_call` 等 link |
| Trace 页签框架 | `frontend/src/tabs/registry.js`、`TraceWorkspace.vue` | 可直接新增独立 tab 并按需加载 |

现有 trajectory 分类作用域是：

```text
(trace_id, process identity, classifier_id)
```

因此不能只按进程 PID 或模型名合并 trajectory。前端 lane 的稳定身份应使用后端返回的 `trajectory_id`，统计时则以相同的 scope 作为基准。

## 3. 范围

### 3.1 第一阶段（MVP）

- 新增 Trace 页签 `LLM Trajectory`；
- 一次请求取得一个 trace 内的全部 LLM trajectory 节点和强关系；
- 节点展示时间、模型、block 数、user message 数、tool result 数、状态；
- 展示严格前缀延续和严格前缀分叉；
- 根据 `start_reason` 支持展示 compaction boundary，但不在 Web 层自行猜测；
- 展示原始计数和定义明确的统计指标；
- 点击节点复用现有 action detail API 打开详情；
- 支持本地 SQLite 和 cluster root 两种运行方式。

### 3.2 后续阶段

- 内容相关但非严格前缀的虚线关系；
- compaction/上下文改写识别器；
- subagent 语义标签和跨进程 lane 分组；
- 大型 trace 的游标分页或时间窗口查询。

### 3.3 非目标

- 不在浏览器中读取 canonical request body 后重新执行 trajectory 分类；
- 不把布局坐标持久化到 SQLite；
- 不修改已有 `trajectory_id` 的含义；
- 不把“当前 response 产生的 tool call 数”错误地当成“请求历史内的 tool result 数”。

## 4. 关系语义

后端输出以下关系：

| edge kind | 数据来源 | 含义 | MVP 样式 |
| --- | --- | --- | --- |
| `append` | `parent_action_id` | child 历史以 parent 历史为严格前缀，并继续同一 trajectory | 实线、同 lane |
| `fork` | `forked_from_action_id` | child 从已有节点的严格前缀分叉，形成新 trajectory | 实线、跨 lane |
| `related` | 后续相关性识别器 | 内容相关但不是完整前缀 | 虚线，MVP 不返回 |

`duplicate_root` 表示完整历史重复。当前 lineage 模型没有保存它所重复的源节点，因此 MVP 可以统计并标记该节点，但不能画出可靠的 duplicate source edge。若后续需要这条边，应新增显式关系字段或通用 trajectory relation 记录，不能按时间就近连接。

`ContextRewriteOrCompression` 和 `RuntimeReset` 已在领域枚举与 SQLite code 中预留，但当前分类器尚未产生这两种原因。因此：

- API 要按已有 `start_reason` 原样输出；
- UI 遇到 `context_rewrite_or_compression` 时，可在该 root 前插入一个仅用于展示的 compaction marker；
- 第一阶段的真实数据通常不会出现 marker；
- compaction 检测必须在 semantic runtime 分类阶段实现，不能在 Web API 中实现。

## 5. 后端 API 设计

### 5.1 新接口

```http
GET /api/traces/{trace_id}/llm-trajectories
```

该路径与已有单 trajectory 接口兼容：

```http
GET /api/traces/{trace_id}/llm-trajectories/{trajectory_id}
```

MVP 不增加查询参数。若后续单 trace 节点量明显增大，再增加 `from_nanos`、`to_nanos` 和游标，不应一开始引入 offset 分页，因为分页可能切断父子边。

### 5.2 响应示例

```json
{
  "trace_id": 42,
  "partial": false,
  "nodes": [
    {
      "id": "llm-request-a",
      "trajectory_id": "llm-request-a",
      "trajectory_position": 0,
      "transition": "root",
      "start_reason": "unspecified",
      "inference_version": 2,
      "start_time": 1787713811120,
      "start_time_unix_nanos": "1787713811120000000",
      "model": "deepseek-v4-flash",
      "classifier_id": "deepseek.com",
      "block_count": 30,
      "user_message_count": 1,
      "tool_result_count": 3,
      "process": {
        "process_id": 1200
      },
      "status": "success",
      "completeness": "complete"
    }
  ],
  "edges": [
    {
      "source": "llm-request-a",
      "target": "llm-request-b",
      "kind": "append",
      "confidence": "derived"
    }
  ],
  "stats": {
    "node_count": 9,
    "trajectory_count": 3,
    "append_count": 5,
    "fork_count": 2,
    "duplicate_count": 1,
    "strongly_linked_node_ratio": 0.7778,
    "duplicate_node_ratio": 0.1111
  },
  "capabilities": {
    "strict_prefix_edges": true,
    "related_edges": false,
    "compaction_detection": false
  }
}
```

字段约定：

- 所有计数缺失时返回 `null`，不要用 `0` 掩盖 retention 配置未保留该信息；
- `start_time` 沿用现有 API 的 Unix 毫秒数，`start_time_unix_nanos` 沿用字符串编码并用于稳定排序；
- `process` 沿用现有 action JSON 的 process 结构，不另造字符串 ID；
- `partial` 表示响应是否因为容量、分页或不支持的存储后端而截断，不表示某个 action 的 semantic completeness；
- `capabilities` 让前端明确知道虚线和 compaction 是否可用，避免通过空数组猜测能力。

### 5.3 指标定义

第一阶段优先展示原始计数。比例必须固定定义：

```text
strongly_linked_node_ratio = (append_count + fork_count) / node_count
duplicate_node_ratio       = duplicate_count / node_count
```

Issue 草图中的“前缀匹配率”容易产生歧义：首个 root 天然没有父节点，`duplicate_root` 又是精确匹配但没有 source edge。MVP UI 建议使用“强关联节点比例”，不要直接命名为“前缀匹配率”。如果产品必须使用前缀匹配率，需要先确定是否排除每个 scope 的首节点、是否把 duplicate 计入命中，再把公式写入 API 版本约定。

### 5.4 错误行为

- trace ID 非法：`400 Bad Request`；
- trace 不存在：沿用当前 trace API 行为；
- trace 已 purge 或 SQLite 查询失败：当前边界可先保持 `400` 兼容，但实现时建议逐步区分为 `404`/`500`；
- cluster 中非 SQLite 或不支持 semantic lineage 的 shard：返回空图且 `partial: true`，并在 capability 中说明不可用，而不是伪装成完整空图。

## 6. 后端实现方案

### 6.1 增加批量读取契约

现有接口只能按 action、trajectory 或 fork source 查询。若为整张图逐个调用 `llm_request_lineage` 会形成 N+1 查询。

在以下 trait 增加方法：

```rust
fn llm_request_lineages(
    &self,
    trace_id: TraceId,
) -> Result<Vec<LlmRequestLineage>, SemanticActionStoreError>;
```

修改位置：

1. `crates/contracts/semantic_action/src/store.rs`
2. `crates/storage/core/src/backend.rs`
3. `crates/storage/adapters/sqlite/src/backend.rs`
4. `crates/storage/adapters/sqlite/src/semantic_actions/store/mod.rs`
5. `crates/storage/adapters/sqlite/src/semantic_actions/llm_request_lineage.rs`

SQLite 实现复用 `query_many` 和 `select_sql`：

```sql
WHERE lineage.trace_id = ?1
ORDER BY request_action.start_time_ns ASC,
         lineage.trajectory_root_action_key ASC,
         lineage.trajectory_position ASC
```

当前 `select_sql` 没有 join `semantic_actions` 的时间字段。可以让存储层仍按 trajectory/position 返回，由 Web 聚合层按已读取的 request action `start_time` 排序；这样不扩大 lineage DTO，也避免 SQL 与 action schema 字段耦合。必须保证相同时间戳时以 `action_id` 作稳定 tie-break。

现有 `llm_request_lineage` 表已具备 `trace_id`，MVP 无需 schema migration。若真实查询计划表明全 trace 扫描慢，再补 `(trace_id, trajectory_root_action_key, trajectory_position)` 索引；不要在没有基准数据时提前加索引。

### 6.2 新增 Web read model

建议新增独立文件：

```text
crates/apps/web/src/view/llm_trajectory.rs
```

不要继续把聚合逻辑堆进已经较大的 `view/actions.rs`。模块内部定义可序列化 DTO，并使用 `serde::Serialize` + `serde_json::to_string`，复杂嵌套响应不建议手工拼 JSON。

聚合过程：

1. `llm_request_lineages(trace_id)` 读取全量 lineage；
2. `semantic_actions_matching_kinds_lite(trace_id, &["llm.request"])` 批量读取 request action；
3. 以 `action_id` 构造 action map；
4. 对每条 lineage 生成 node；
5. `parent_action_id` 生成 `append` edge；
6. `forked_from_action_id` 生成 `fork` edge；
7. 按 `(start_time, action_id)` 排序 nodes；
8. 计算 stats 和 capabilities；
9. 检查悬空引用：缺 action 的 lineage 不应 panic，应跳过节点并将 `partial` 置为 `true`。

MVP 不需要为了 node 计数读取 response/tool action。图中 `response tool` 表示该 request 输入历史内已有多少 tool result，而不是这个 request 的 response 随后生成多少 tool call。该计数应和 `block_count`、`user_message_count` 一样在请求投影时生成。

### 6.3 新增 `tool_result_count` 属性

新增属性键：

```rust
pub const TOOL_RESULT_COUNT: &str = "llm.request.tool_result_count";
```

修改位置：

- `crates/contracts/semantic_action/src/attr_keys.rs`
- `crates/core/semantic_action_runtime/src/llm_pipeline/projection/retention/request_blocks/metadata.rs`
- `crates/core/semantic_action_runtime/src/llm_pipeline/projection/retention/request_blocks.rs`
- `crates/core/semantic_action_runtime/src/llm_pipeline/projection/projector/request.rs`

计数规则应覆盖当前已经识别的 provider 形态：

- Anthropic：`role=user` 且 content block type 为 `tool_result`/`tool-result`，按 block 计数；
- OpenAI Chat：`role=tool` 的 message，按 message 计数；
- Responses API：`input` 中 `type=function_call_output` 或等价的已规范化 tool output item，按 item 计数。

建议把判断函数与当前 `message_is_user_input` 共享底层 helper，防止同一个 tool result 一边被计为 user message，一边又被计为 tool result。`Shape` 与 `CanonicalBlocks` retention 都可以从当次解析的 JSON 计算该值；`None` retention 返回缺失属性。

为区分“真实计数为 0”和“未采集”，还应把内部
`RequestContentMetadata.user_message_count` 从 `usize` 调整为 `Option<usize>`，并让新增的
`tool_result_count` 同样使用 `Option<usize>`：`Shape`/`CanonicalBlocks` 写入 `Some(0)`，
`None` retention 写入 `None`。生成 attributes 时对 `Some(0)` 也必须持久化字符串 `"0"`。

历史数据库没有该属性是允许的，API 返回 `null`，不需要数据迁移。

### 6.4 接入 HTTP 路由

当前 `http.rs` 对 cluster root 和 local storage 分别匹配路由，两个分支都需要增加：

```rust
[trace_id, "llm-trajectories"] => { ... }
```

调用链建议为：

```text
http.rs
  -> view::llm_trajectory_graph_json(...)
     -> view::llm_trajectory::graph_json(...)

cluster mode:
http.rs
  -> view::cluster::llm_trajectory_graph_json(...)
     -> 定位 shard 和 local_trace_id
     -> view::llm_trajectory_graph_json(...)
```

需要修改：

- `crates/apps/web/src/http.rs`
- `crates/apps/web/src/view.rs`
- `crates/apps/web/src/view/cluster.rs`
- 新增 `crates/apps/web/src/view/llm_trajectory.rs`

### 6.5 不建议的实现

- 不要让前端先拉全部 actions，再调用每个 trajectory endpoint；
- 不要在 Web API 内重新读取 canonical body 做 prefix 比较；
- 不要用颜色表达 trajectory 身份并由后端持久化颜色；
- 不要从相邻时间节点推断 fork/compaction；
- 不要把 invalid semantic action link 当成有效 edge。

## 7. 前端实现方案

### 7.1 文件结构

```text
crates/apps/web/frontend/src/
├── api.js
└── tabs/core/llm-trajectory/
    ├── LlmTrajectoryTab.vue
    ├── LlmTrajectoryGraph.vue
    ├── LlmTrajectoryNode.vue
    ├── model.js
    ├── layout.js
    ├── model.test.js
    └── llm-trajectory.css
```

修改 `tabs/registry.js`：

```js
llmTrajectory: 'llm_trajectory'
```

建议把页签放在 `Action Tree` 后面。修改 `api.js` 增加：

```js
export function readLlmTrajectoryGraph(traceId, { signal } = {}) {
  return fetchJson(
    `/api/traces/${encodeURIComponent(traceId)}/llm-trajectories`,
    { signal },
  );
}
```

### 7.2 加载方式

沿用 `TraceWorkspace.vue` 的按 tab 懒加载模式：

- 新增 `trajectoryGraph` shallow ref；
- 只在 active tab 为 `llm_trajectory` 时调用 API；
- 切换 trace 时清空旧图；
- 使用 token 或 `AbortController` 防止旧 trace 的慢响应覆盖新 trace；
- 将数据通过 `activeTabProps` 传给 tab，不让图组件自己管理全局 trace 选择。

### 7.3 布局算法

MVP 不引入图形库，仓库当前前端只依赖 Vue 和 Lucide，使用 SVG 足够：

1. nodes 按 `(start_time_unix_nanos, id)` 排序确定 Y；
2. 第一条 trajectory 占 lane 0；
3. fork trajectory 分配当前可用的新 lane；
4. lane X 固定为 `left + lane_index * lane_width`；
5. append edge 画同 lane 竖直实线；
6. fork edge 使用三次贝塞尔曲线连接 source 与新 lane root；
7. 节点使用两级文本：标题展示 `Tn · Step n · model`，元数据展示
   `block/user message/tool result` 计数及相对上一请求的增量；
8. trajectory 颜色由 `trajectory_id` 的稳定 hash 映射到主题色板，颜色只用于辅助，边类型还必须由线型/图例表达；
9. 点击节点调用现有 `readActionDetail`，复用 `DetailPanel.vue`。

应优先支持纵向滚动和横向 lane 滚动，不要为了把所有节点压进一屏而缩小文字。节点超过约 500 个时可再考虑虚拟化；第一阶段先用真实 trace 做性能基准。

### 7.4 信息区

左侧信息区展示：

- node/trajectory/append/fork/duplicate 原始数量；
- 强关联节点比例；
- 重复节点比例；
- 当前能力提示，例如“仅展示严格前缀关系，内容相关虚线关系尚未启用”。

当 `tool_result_count` 为 `null` 时显示 `—` 并提示“当前 retention 未保留该统计”，不要显示 0。

### 7.5 节点详情字段

泳道中的一个节点表示一次 `llm.request` 快照，不表示一轮完整问答。点击节点后，
右侧默认使用 `Focused` 模式展示适合日常分析的字段：

| 字段或区域 | 含义 |
| --- | --- |
| `Time` | 请求开始时间 |
| `Trajectory` | 连续上下文的稳定内部 ID；图中的 `T1`、`T2` 是可读简称 |
| `Position` | trajectory 内从 0 开始的位置；图中的 Step 从 1 开始 |
| `Transition` | `root`、`append`、`fork_root` 或 `duplicate_root` |
| `Start reason` | 新上下文原因，例如 `unspecified`、运行时重置或上下文压缩 |
| `model` / `provider` | 模型和供应商 |
| `messages` | 当前请求中解析出的消息数量 |
| `blocks` | 规范化请求内容块数量，不等同于对话轮数 |
| `bytes` | 规范化请求体大小 |
| `Last message` / `Message preview` | 发给模型的最新消息或采集阶段保留的截断预览，不是本次模型回答 |
| `Message context` | 当前请求携带的消息历史，按最新消息优先展示 |
| `Tool results in context` | 已执行并进入上下文的工具结果累计数；`+N` 表示相对上一请求的新增量 |
| `Available tool definitions` | 模型本次可以选择调用的工具定义，不表示这些工具已经执行 |

`tool result` 与 `available tool definition` 必须分开理解。例如“2 results、10
definitions”表示上下文已有两个工具返回结果，同时模型被声明可以使用十种工具。

### 7.6 节点详情操作与完整数据

- `Load request insights` 首次按需加载完整 request content（当前上限 128 KiB），使消息、
  tool result 和工具定义摘要更完整；加载后按钮变为 `Hide request insights`，隐藏后可通过
  `Show request insights` 使用前端缓存重新展开，不会重复请求后端。
- `Focused / All data` 开关位于详情最下方。默认 `Focused` 展示基础字段、错误和语义摘要；
  `All data` 额外展示 canonical request body、Attributes、Payload、Path Set 和原始 JSON。
- `Canonical request body` 是实际模型请求的规范化结构，使用懒加载 JSON 树，适合检查
  messages、tools、system prompt 和供应商参数。
- `Attributes` 是为查询和分析提取的扁平属性；`JSON` 是完整 action 记录，包含 ID、时间、
  process、status、completeness、attributes 和 evidence，主要用于底层排障。
- `status` 表示 action 成功或失败，`completeness` 表示证据是否完整；二者与图接口的
  `partial` 不同，`partial` 表示整张图是否因存储或读取能力而不完整。

## 8. Compaction 与虚线关系的后续设计

### 8.1 Compaction

后续检测应放在：

```text
crates/core/semantic_action_runtime/src/llm_pipeline/projection/trajectory/
```

检测器输入应使用内存中的 `HistoryAtom` 和 provider context，不依赖 Web 或已脱敏的展示属性。只有达到明确阈值后，才生成：

- 新 trajectory root；
- `start_reason = context_rewrite_or_compression`；
- 指向来源节点的显式弱关系。

当前 `LlmRequestLineage` 只允许 `forked_from_action_id` 表达强 fork。不要复用该字段承载弱相关关系。建议后续新增通用 relation 模型，例如：

```text
LlmTrajectoryRelation {
    source_action_id,
    target_action_id,
    kind: content_related | compaction,
    score,
    inference_version
}
```

再投影为 `related` 虚线 edge。

### 8.2 Subagent

当前 trajectory scope 含 process identity，所以不同进程的 subagent request 通常不会因为请求前缀自动合并。后续可利用已有：

- `llm.tool_call.agent_invocation`
- `agent.invocation.child_llm_request`

把 agent invocation 关系投影为独立的 `subagent` edge。该边和内容前缀关系语义不同，前端不能统一标成 fork。

## 9. 测试计划

### 9.1 Storage

- 一个 trace 批量读取多个 trajectory，顺序稳定；
- 不返回其他 trace 的 lineage；
- purge 后错误与现有方法一致；
- 空 trace 返回空 Vec；
- parent/fork 引用完整。

### 9.2 Web read model/API

- root + 两次 append 生成 3 nodes、2 append edges；
- 一个 fork root 生成跨 trajectory fork edge；
- duplicate root 增加 duplicate count，但不生成虚假 edge；
- request action 缺失时 `partial=true` 且不 panic；
- 缺失 block/user/tool count 序列化为 `null`；
- local 和 cluster mode 的 JSON 契约一致；
- 节点相同时间戳时按 action ID 稳定排序；
- 特殊字符 action ID 正确 JSON 转义。

### 9.3 Request projection

- Anthropic tool result block 计数；
- OpenAI `role=tool` 计数；
- Responses API function output 计数；
- 普通 user message 不计入 tool result；
- tool-only user content 不重复计入 user message；
- retention `None` 不写入伪造的 0。

### 9.4 Frontend

- `api.test.js` 验证 trace ID 编码和请求路径；
- `model.test.js` 验证 lane 分配、稳定颜色、节点/edge 排序；
- 空图、单节点、分叉、多 trajectory、缺失计数；
- 快速切换 trace 时旧响应不覆盖新图；
- 点击节点可打开 action detail；
- 深色/浅色主题下节点和边均可辨认。

## 10. 验收标准

第一阶段完成需同时满足：

1. 一个 trace 只发起一次 graph API 请求即可渲染完整强关系图；
2. append 与 fork 关系和 `llm_request_lineage` 数据逐条一致；
3. 不生成未经 classifier 确认的虚线、compaction 或 duplicate source edge；
4. 节点时间排序稳定，刷新页面后 lane 和颜色不跳变；
5. block/user/tool result 缺失与零值可区分；
6. local 和 cluster mode 都可使用；
7. 500 个节点的测试 trace 中，API 和首屏渲染没有明显卡顿；
8. Storage、Web 聚合和前端 model 都有自动化测试。

## 11. 推荐开发顺序

1. 增加 `tool_result_count` 投影属性及单元测试；
2. 增加 `llm_request_lineages(trace_id)` 存储契约和 SQLite 实现；
3. 实现独立 Web read model 与 `GET /llm-trajectories`；
4. 补齐 local/cluster 路由和 API 测试；
5. 前端增加 API 方法、tab 注册和懒加载状态；
6. 实现纯函数 `model.js`/`layout.js` 及测试；
7. 实现 SVG 图、统计信息区和详情联动；
8. 使用真实多分支 Agent trace 做性能与语义验收；
9. 单独立项实现 compaction、内容相关虚线和 subagent edge。

按照这个顺序，第一阶段不会被弱相关识别算法阻塞，并且每一步都能独立测试和评审。
