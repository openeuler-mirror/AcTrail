# LLM Trajectory 图

LLM Trajectory 图在一个 trace 内展示 LLM request 节点、延续与分叉关系、重复统计及请求详情。本地 Web 与集群 Web 均提供该视图。图消费语义运行时已经持久化的 lineage，查询层与浏览器负责投影和展示。

## 身份与所有权

| 对象 | 权威来源与含义 |
| --- | --- |
| request 节点 | 持久化 `llm.request` action，节点 ID 为 `action_id` |
| 分类作用域 | `(trace_id, ProcessIdentity, classifier_id)`，进程使用内部稳定身份 |
| `trajectory_id` | 分类器分配的轨迹身份；append 延续已有轨迹，root、fork 与 duplicate 可建立新轨迹 |
| 关系与位置 | `LlmRequestLineage` 保存 parent、fork source、transition、start reason 与 trajectory position |
| 页面数据 | `TraceWorkspace` 保存当前 trace 的加载结果；图组件保存选择与详情状态 |
| lane、颜色与位置 | 前端 `model.js` 从返回节点派生；T1/T2 等编号属于展示属性 |

运行时分类器拥有分类状态，Web 使用持久化 lineage。强关系可以来自请求历史前缀，也可以来自已确认的 provider `previous_response_id` 绑定。

## 关系与统计

`parent_action_id` 生成 `append` 边；没有 parent 时，`forked_from_action_id` 生成 `fork` 边。当前关系 confidence 为 `derived`。缺少关联 action 或边端点时省略不可建立的部分并返回 `partial=true`。

`duplicate_root` 仍作为节点显示并计数，transition 在提示与详情中可见；目前 lineage 未保存重复源节点身份。`context_rewrite_or_compression` 可投影为 `compaction_boundary` 并显示圆环，但当前分类器尚未生成该原因。

| 统计 | 含义 |
| --- | --- |
| `node_count` | 成功关联并返回的 request 节点数 |
| `trajectory_count` | 返回节点中的不同 trajectory ID 数 |
| `append_count`、`fork_count`、`duplicate_count` | transition 分别为 append、fork_root、duplicate_root 的节点数 |
| `strongly_linked_node_ratio` | `(append_count + fork_count) / node_count` |
| `duplicate_node_ratio` | `duplicate_count / node_count` |

空图的比例为零。transition 计数可能大于对应边数，例如父节点缺失时节点仍计数。block、user message 和 tool result 计数缺失时返回 `null`；tool result 指请求上下文中已有的工具结果。

## 查询与性能边界

| 接口 | 职责 |
| --- | --- |
| `GET /api/traces/{trace_id}/llm-trajectories` | 查询整张图 |
| `GET /api/traces/{trace_id}/llm-trajectories/{trajectory_id}` | 查询单条轨迹的 lineage |
| `GET /api/traces/{trace_id}/actions/{action_id}/lineage/llm-request` | 查询单 action lineage |

整图一次读取当前 trace 的 lineage 和 `llm.request` lite actions，通过 action map 关联并排序。lite 查询省去 evidence hydration，仍读取 action 属性及所需冷字段。前端生成完整 SVG；数据量、内存和绘制成本随节点数增加。当前整图接口没有分页、节点上限或专用预算。

页签首次激活时加载；切换 trace 或刷新时清空缓存，加载 token 防止旧响应覆盖新 trace。同一 trace 已加载的数据复用。点击节点使用既有 action detail API，正文按需读取。

本地接口使用本地 trace ID；集群入口先按持久化 UID 定位 snapshot，再转换内部身份并投影外部表示。缺少可用 SQLite snapshot 时返回空图、`partial=true`，并关闭各项 capability。

## 能力与源码

有可用数据源时，接口声明 `strict_prefix_edges=true`、`related_edges=false`、`compaction_detection=false`。`strict_prefix_edges` 是当前接口字段名，强关系来源也包括已确认 provider binding。

| 实现 | 路径 |
| --- | --- |
| 运行时分类 | [trajectory classifier](../../../crates/core/semantic_action_runtime/src/llm_pipeline/projection/trajectory/classifier/implementation.rs) |
| 图聚合、统计与 marker | [llm_trajectory.rs](../../../crates/apps/web/src/view/llm_trajectory.rs) |
| 集群视图 | [cluster.rs](../../../crates/apps/web/src/view/cluster.rs) |
| 页面状态 | [TraceWorkspace.vue](../../../crates/apps/web/frontend/src/workspaces/TraceWorkspace.vue) |
| 图组件与派生布局 | [LlmTrajectoryTab.vue](../../../crates/apps/web/frontend/src/tabs/core/llm-trajectory/LlmTrajectoryTab.vue)、[model.js](../../../crates/apps/web/frontend/src/tabs/core/llm-trajectory/model.js) |
