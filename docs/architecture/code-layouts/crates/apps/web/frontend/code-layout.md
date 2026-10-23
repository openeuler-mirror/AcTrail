状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/web/frontend/ # 将查询接口和管理操作组织为 Vue 工作区、trace 视图及按需详情。
├── [x] package.json # 声明 Vue、Vite 与 lucide 依赖和前端构建入口。
├── [x] vite.config.js # 配置构建产物名称和静态资源输出。
└── [x] src/ # 应用状态、工作区、trace 视图和共享展示组件。
    ├── [x] main.js # 创建 Vue 应用并挂载根组件。
    ├── [x] App.vue # 持有 activeWorkspace、主题、语言、刷新及通知状态，协调跨工作区导航。
    │   ├── [x] activeWorkspace # 根级当前工作区状态，控制主内容切换。
    │   ├── [x] pendingTraceSelection # 将其他工作区选中的 trace 交给 trace 工作区。
    │   └── [x] refresh # 驱动当前工作区刷新并维护全局刷新状态。
    ├── [x] api.js # 封装查询与管理 HTTP 请求、参数编码和取消信号。
    ├── [x] styles.css # 定义应用壳几何、组件基础样式和响应式断点。
    ├── [x] workspaces/ # 组织 trace、统计、配置、插件和告警的页面级状态。
    │   ├── [x] TraceWorkspace.vue # 拥有所选 trace、详情、tab 和 TimelineWindow，协调异步查询与视图切换。
    │   │   ├── [x] selectedTraceId # 当前 trace 选择，作为详情与子视图查询入口。
    │   │   ├── [x] timeline # TimelineWindow 实例，向时间线相关视图提供窗口数据。
    │   │   └── [x] tabDefinitionsById # 按注册表索引叶视图描述，用于 tab 选择与组件绑定。
    │   ├── [x] DashboardWorkspace.vue # 展示整体活动和概览统计。
    │   ├── [x] StatsWorkspace.vue # 协调各统计子工作区的切换。
    │   ├── [x] AlertsWorkspace.vue # 管理告警筛选、列表和详情选择。
    │   ├── [x] ConfigWorkspace.vue # 展示运行配置并提交允许编辑的配置项。
    │   ├── [x] PluginsWorkspace.vue # 管理安装包、运行实例、加载卸载及插件操作。
    │   ├── [x] runtime.css # 定义工作区内共用布局与状态样式。
    │   ├── [x] locale/ # 保存工作区导航和通用操作文案。
    │   ├── [x] traces/ # 管理 trace 列表过滤与分页。
    │   │   ├── [x] pages.js # 维护 trace 页查询和翻页状态。
    │   │   ├── [x] filters.js # 规范化 trace 列表过滤参数。
    │   │   └── [x] TraceListControls.vue # 提供 trace 列表筛选与翻页控件。
    │   ├── [x] timeline/ # 按可见时间窗口读取动作并限制前端缓存量。
    │   │   ├── [x] window.js # 协调当前 trace、过滤条件、可见范围和取消中的请求。
    │   │   │   └── [x] class TimelineWindow # 拥有响应式窗口数据、缓存和 AbortController。
    │   │   │       ├── [x] load # 刷新 trace 或过滤条件并读取可见窗口。
    │   │   │       ├── [x] setViewport # 按平移缩放后的范围安排按需查询。
    │   │   │       └── [x] cancel # 取消定时器与在途请求。
    │   │   ├── [x] cache.js # 缓存完整窗口并按条目预算逐出。
    │   │   │   └── [x] class TimelineWindowCache # 拥有历史窗口，提供覆盖判断、投影和预算检查。
    │   │   ├── [x] filters.js # 将动作类型、进程和时间条件转换为窗口查询。
    │   │   ├── [x] range.js # 计算时间边界、可见范围和预取范围。
    │   │   ├── [x] constants.js # 定义窗口、请求和缓存预算默认值。
    │   │   ├── [x] utils.js # 提供窗口数据处理所需的共用操作。
    │   │   └── [x] TimelineControls.vue # 展示窗口查询状态与筛选操作。
    │   ├── [x] plugins/ # 渲染插件加载参数、授权范围、配置与命令表单。
    │   │   ├── [x] PluginDetailPanel.vue # 展示所选插件包或实例详情。
    │   │   ├── [x] PluginConfigPanel.vue # 管理插件配置编辑与提交。
    │   │   ├── [x] PluginCommandForm.vue # 根据命令描述组织参数并提交调用。
    │   │   ├── [x] PolicyScopeEditor.vue # 编辑策略授权适用范围。
    │   │   ├── [x] locale/ # 保存插件管理操作与错误文案。
    │   │   └── [x] config/ # 将插件 JSON Schema 转换为动态表单结构。
    │   │       └── [x] PluginConfigObjectSchema.js # 根据当前值解析属性、必填项和条件分支。
    │   │           └── [x] class PluginConfigObjectSchema # 持有生效的对象 schema，裁剪隐藏分支并清理提交值。
    │   ├── [x] dashboard/ # 汇总活动趋势和筛选控件。
    │   │   ├── [x] activity.js # 将活动统计结果转换为看板展示数据。
    │   │   └── [x] ActivityControls.vue # 提供活动时间范围和维度选择。
    │   └── [x] stats/ # 组织模型请求、token、agent 和时间归因统计。
    │       ├── [x] tokenModel.js # 将 token 统计响应转换为共享展示模型。
    │       ├── [x] agent/ # 展示 agent 调用构成、输入分布和 trace 时间线。
    │       │   └── [x] AgentStatsWorkspace.vue # 协调 agent 统计查询和图表选择。
    │       ├── [x] llm/ # 展示模型调用概览、延迟、趋势和探索结果。
    │       │   └── [x] LlmRequestsWorkspace.vue # 管理模型请求统计页面与查询条件。
    │       ├── [x] token/ # 展示 token 汇总、趋势、模型分布和请求明细。
    │       │   └── [x] filters/ # 提供 token 统计维度的多选筛选控件。
    │       ├── [x] charts/ # 提供分类分布、趋势图和绘图计算。
    │       ├── [x] visualizations/ # 将时间桶和类别统计渲染为表格与控件。
    │       └── [x] time-attribution/ # 展示跨 trace 时间归因与范围过滤。
    │           ├── [x] TimeAttributionWorkspace.vue # 管理归因汇总与明细查询状态。
    │           └── [x] locale/ # 保存归因统计标签和问题提示文案。
    ├── [x] tabs/ # 根据统一注册表将单 trace 数据映射为叶视图。
    │   ├── [x] registry.js # 定义 TAB_DEFINITIONS 与 TAB_GROUP_DEFINITIONS，绑定视图标识、分组及组件。
    │   ├── [x] tableModel.js # 将列表记录转换为可排序和筛选的表格行。
    │   ├── [x] tableConfig.js # 配置各类表格列和展示选项。
    │   ├── [x] TableTraceTab.vue # 提供各类 trace 列表共用的表格容器。
    │   ├── [x] shared/ # 复用工具调用等跨 tab 的显示投影。
    │   ├── [x] core/ # 展示动作、命令、模型轨迹及多种时间视图。
    │   │   ├── [x] overview/ # 汇总 trace 基本信息与关键指标。
    │   │   ├── [x] action-tree/ # 根据动作关系构造树并展示上下游动作。
    │   │   │   ├── [x] ActionTreeTab.vue # 管理动作树展开、选择和详情联动。
    │   │   │   └── [x] model.js # 将动作与关系边整理为展示树。
    │   │   ├── [x] commands/ # 展示命令执行和关联输入输出。
    │   │   ├── [x] llm-trajectory/ # 展示模型请求、工具调用与返回轨迹。
    │   │   ├── [x] timeline/ # 将进程与动作渲染为顺序时间线。
    │   │   ├── [x] time-attribution/ # 展示单 trace 的时间分类、覆盖率和问题。
    │   │   ├── [x] time-navigation/ # 统一时间视图的缩放、平移和范围限制。
    │   │   │   └── [x] useTimelineNavigation.js # 管理时间导航交互与视口更新。
    │   │   ├── [x] waterfall/ # 渲染动作瀑布图并关联空闲区间和详情。
    │   │   │   ├── [x] WaterfallTab.vue # 协调瀑布图布局、选中项和视口联动。
    │   │   │   ├── [x] model/ # 计算动作层次、显示字段和绘制模型。
    │   │   │   ├── [x] view/ # 管理瀑布图展示、详情和交互状态。
    │   │   │   └── [x] idle/ # 分页读取并展示空闲区间。
    │   │   └── [x] flame-graph/ # 将动作时间区间转换为火焰轨道并按画布绘制。
    │   │       ├── [x] FlameGraphTab.vue # 管理火焰图的选择、范围和轨道视图。
    │   │       ├── [x] canvas-renderer.js # 绘制当前可见的轨道与区间。
    │   │       └── [x] model/ # 将动作、工具影响和时间关系整理为火焰图帧。
    │   ├── [x] activity/ # 展示事件、文件、网络和负载列表。
    │   │   ├── [x] events/ # 展示原始事件及事件详情。
    │   │   ├── [x] files/ # 展示文件访问与路径相关事实。
    │   │   ├── [x] network/ # 展示连接与网络活动。
    │   │   └── [x] payloads/ # 展示负载段并按需读取正文。
    │   └── [x] system/ # 展示 trace 的进程、资源、诊断和告警。
    │       ├── [x] processes/ # 展示进程成员列表和身份信息。
    │       ├── [x] process-tree/ # 构造进程父子关系树。
    │       ├── [x] resources/ # 展示 trace 工作负载的资源采样。
    │       ├── [x] diagnostics/ # 展示采集与处理诊断。
    │       └── [x] alerts/ # 展示关联 trace 的告警。
    ├── [x] components/ # 提供详情、表格、协议洞察和递进内容加载控件。
    │   ├── [x] DetailPanel.vue # 展示所选记录的详情容器。
    │   ├── [x] DataTable.vue # 提供共享行表格与选择交互。
    │   ├── [x] JsonTree.vue # 展示 JSON 内容的树形结构。
    │   ├── [x] json/ # 延后创建尚未展开的 JSON 子节点。
    │   ├── [x] llm/ # 按需读取并渲染模型请求块与响应内容。
    │   │   ├── [x] NodeReader.js # 对单个动作的内容节点请求实施并发限制和统一取消。
    │   │   │   └── [x] class NodeReader # 拥有请求队列、并发计数和 AbortController。
    │   │   │       ├── [x] read # 将内容节点查询放入有界并发队列。
    │   │   │       └── [x] close # 取消在途读取并拒绝剩余队列请求。
    │   │   ├── [x] LlmCallPanel.vue # 组合模型调用请求、响应和元数据详情。
    │   │   └── [x] LlmContentNode.vue # 递进展示按需加载的模型内容节点。
    │   ├── [x] http/ # 展示 HTTP 请求内容与负载正文。
    │   ├── [x] markdown/ # 将消息中的 Markdown 内容渲染为详情预览。
    │   ├── [x] navigation/ # 提供当前视图的导航条与范围操作。
    │   ├── [x] timeline/ # 提供时间线概览与范围定位组件。
    │   ├── [x] time-attribution/ # 共享时间分摊条与分类展示模型。
    │   └── [x] locale/ # 保存共享组件按钮、状态和提示文案。
    ├── [x] command/ # 将命令动作整理为命令行、启动上下文与洞察数据。
    ├── [x] http/ # 将 HTTP 事件和负载整理为请求响应洞察。
    ├── [x] mcp/ # 分类 JSON-RPC 消息，关联负载证据并生成 MCP 洞察。
    ├── [x] llm/ # 将模型调用事实整理为模型请求与响应洞察。
    ├── [x] detail/ # 为通用详情面板选择与组合协议洞察。
    ├── [x] composables/ # 复用对话框、展开与焦点管理行为。
    ├── [x] locale/ # 管理语言选择及中英文界面资源。
    │   ├── [x] index.js # 注册语言资源并提供文案查询入口。
    │   ├── [x] en-US.js # 英文公共文案。
    │   └── [x] zh-CN.js # 中文公共文案。
    └── [x] theme/ # 根据主题选择加载角色 token 并维持共享样式契约。
        ├── [x] index.js # 解析 system、light、white 和 dark 主题模式。
        ├── [x] contract.css # 定义组件使用的主题角色和通用样式变量。
        ├── [x] dark/ # 深色主题 token。
        ├── [x] white/ # 白色主题 token。
        └── [x] light/ # 浅色主题 token。
```
