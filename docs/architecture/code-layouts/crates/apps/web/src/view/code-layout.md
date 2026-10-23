状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/web/src/view/ # 将存储事实投影为界面所需 JSON，组织列表分页、内容按需读取和统计聚合。
├── [x] traces.rs # 查询 trace 列表与详情。
├── [x] traces/ # 处理 trace 列表分页与查询结果窗口。
│   └── [x] page.rs # 组合单页 trace 元数据与分页信息。
├── [x] events.rs # 汇总事件数量并生成事件列表。
│   └── [x] struct EventCounts # 汇总按事件类别统计的计数。
├── [x] actions.rs # 读取动作及关联边并构造动作详情。
├── [x] actions/ # 将动作内容与 LLM、工具显示数据分开投影。
│   ├── [x] serialization.rs # 将语义动作序列化为界面记录。
│   ├── [x] llm_call_display.rs # 构造模型调用的展示字段。
│   └── [x] llm_tool_display.rs # 构造工具调用与返回的展示字段。
├── [x] action_content.rs # 按内容节点与分页请求读取动作正文。
├── [x] action_tree_roles.rs # 将动作关系角色转换为动作树连接含义。
├── [x] payloads.rs # 查询负载段元数据和按需读取的正文。
├── [x] commands.rs # 将命令执行动作投影为命令详情和关联记录。
├── [x] llm_trajectory.rs # 组合模型请求、工具调用和关系边形成交互轨迹。
├── [x] timeline.rs # 查询给定窗口内的动作、上下文与关联关系。
├── [x] topology.rs # 构造进程与动作的时间线拓扑。
│   └── [x] struct TimelineRow # 保存时间线展示行的身份和时间信息。
├── [x] agent_idle.rs # 根据活动区间计算 agent 空闲区间及分页结果。
├── [x] time_attribution.rs # 定义时间归因的区间、覆盖率、结果和终态缓存数据。
│   ├── [x] struct TraceAttribution # 汇总单 trace 的时间分摊、范围、覆盖率与问题。
│   ├── [x] struct AttributionSegment # 表达归因分类后的时间片段及关联信息。
│   ├── [x] struct TerminalCacheKey # 以存储路径、trace、终态时间和存储修订标识终态投影。
│   └── [x] struct TerminalProjectionCache # 保存可复用的终态 trace 归因投影。
├── [x] time_attribution/ # 从动作关系计算归因区间，再生成 trace 与跨 trace 汇总。
│   ├── [x] api.rs # 定义范围、维度和明细分页查询参数。
│   │   └── [x] struct TimeAttributionRowsQuery # 承载归因明细筛选与分页条件。
│   ├── [x] model.rs # 将模型调用与工具出现位置整理为归因输入。
│   │   └── [x] struct ModelProjection # 保存模型调用投影所需的区间和关联结果。
│   ├── [x] projection.rs # 从存储动作、关系和进程成员事实建立归因投影。
│   ├── [x] partition.rs # 按时间边界切分区间并确定片段类别。
│   ├── [x] turns.rs # 将用户回合与动作区间关联。
│   ├── [x] agents.rs # 解析 agent 进程范围及包含关系。
│   │   └── [x] struct AgentProcessScope # 表达 agent 对应的进程范围。
│   ├── [x] summary.rs # 汇总单 trace 各类别时长、瓶颈与覆盖率。
│   ├── [x] aggregate.rs # 聚合跨 trace 的归因统计与问题。
│   └── [x] cache.rs # 复用终态投影；运行中 trace 重新投影，查询窗口在投影后裁剪。
├── [x] stats.rs # 组织模型调用统计和导出入口。
├── [x] stats/ # 从调用事实生成 token、延迟、维度分布和趋势。
│   ├── [x] model.rs # 定义统计查询、明细数据集与聚合结果。
│   │   ├── [x] struct LlmUsageDataset # 持有用于统计的模型请求行和覆盖信息。
│   │   ├── [x] struct LlmUsageRow # 表达单次模型请求的身份、token、时间与延迟。
│   │   └── [x] struct LlmLatency # 表达请求延迟度量及可用性。
│   ├── [x] projector.rs # 将语义动作转换为模型请求统计行。
│   ├── [x] activity.rs # 按维度聚合活动、token 与请求指标。
│   ├── [x] activity/ # 组织延迟分布和时间序列聚合。
│   │   └── [x] latency.rs # 计算分组延迟和时间桶分布。
│   ├── [x] time_buckets.rs # 将统计范围划分为时间桶。
│   ├── [x] explore.rs # 根据选择的指标、分组和排序生成探索结果。
│   │   └── [x] struct LlmExploreQuery # 承载探索视图的指标、维度与过滤条件。
│   ├── [x] export.rs # 将统计明细或聚合视图导出为文件数据。
│   ├── [x] legacy_token.rs # 提供 token 用量视图的查询和聚合。
│   └── [x] render.rs # 将统计结果转换为前端响应表示。
├── [x] cluster.rs # 组织集群 trace 与关系图查询。
│   └── [x] struct ClusterTraceRow # 表达集群索引中的 trace 展示记录。
├── [x] cluster/ # 读取集群索引并转换关系图与时间线。
│   ├── [x] index.rs # 读取 bundle 索引并执行 trace 分页查询。
│   │   └── [x] struct ClusterIndex # 管理集群根目录下的查询入口。
│   ├── [x] representation.rs # 将集群关系图转换为 JSON。
│   └── [x] timeline.rs # 生成跨 trace 的时间线展示数据。
├── [x] alerts.rs # 查询告警列表、详情和过滤选项。
├── [x] daemon_status.rs # 将 daemon 控制状态转换为管理界面数据。
└── [x] runtime_config.rs # 读取并表示运行配置与可编辑选项。
```
