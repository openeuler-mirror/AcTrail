# Web 查询与管理接口

本文记录 `actrailweb` 的 Trace 查询、正文分页、配置编辑和插件 catalog 约定。组件关系见 [Web Runtime](../../architecture/components/web-runtime.md)。

| 主题 | 查询入口 |
|---|---|
| 数据查询 | [Trace 列表](#trace-列表) · [Metadata](#metadata) · [时间线](#时间线) · [Idle 区间](#idle-区间) · [正文分页](#正文分页) |
| 配置编辑 | [字段选择](#字段选择) · [读取](#读取配置) · [保存](#保存配置) |
| 插件发现 | [Catalog](#插件-catalog) |
| 服务端上限 | [Web 配置](../configuration/daemon.md#web-与-exportsnapshot) |

## 数据查询

### Trace 列表

`GET /api/traces`

| 参数 | 含义 |
|---|---|
| `page` | 正整数页号，默认 `1`，支持直接跳页 |
| `limit` | 每页数量，默认 `50`，默认上限 `250` |
| `trace_id` | Trace 身份筛选 |
| `name` | 名称完全匹配 |
| `from`、`to` | 创建时间闭区间，Unix 纳秒 |

| 响应或行为 | 约定 |
|---|---|
| `traces`、`page`、`total`、`total_pages` | 当前页列表、页号、过滤后准确总数与总页数 |
| 空结果 | `total_pages` 为 `0` |
| 超过末页 | 返回空列表及实际总数 |
| 非法页号、数量或 offset 溢出 | 返回请求错误 |
| 本地排序 | 创建时间和 trace ID 倒序，排除已清理记录 |
| Cluster 排序 | `trace_uid` 主键升序 |
| 一致性与读取量 | 本地总数与当前页在同一读取事务内查询，仅加载当前页元数据与根进程 |
| 计数与跳页成本 | 精确计数及深页 offset 扫描相应元数据 |
| 数据更新 | 列表实时查询，刷新后更新结果 |

Cluster 列表、metadata 和路由使用持久化 `trace_uid`。UID 中的 `/` 随整个 UID 编码为单个 URL 路径段，服务端切分路径后解码。快照内数字身份通过 `local_id` 展示。导入顺序和重复导入保持已有 UID。创建时间筛选遇到非法持久化时间字符串时返回请求错误。

### Metadata

`GET /api/traces/{id}/metadata`

| 内容 | 含义 |
|---|---|
| Trace 元数据与根进程 | 当前 Trace 及根进程展示信息 |
| `timeline_bounds.start_time_unix_nanos` | 动作开始时间的最小值，纳秒字符串 |
| `timeline_bounds.end_time_unix_nanos` | 动作开始时间的最大值，纳秒字符串 |
| 空范围 | 没有动作或 Cluster 尚无快照时，两个端点均为 `null` |

范围通过开始时间索引的两个端点读取。绘图轴终点结合 Trace 生命周期确定。统计由 `/api/traces/{id}/summary` 单独读取。

### 时间线

`GET /api/traces/{id}/actions/timeline`

#### 请求参数

| 参数 | 含义 |
|---|---|
| `kinds` | 逗号分隔的动作类型，省略或为空时查询全部类型 |
| `process_id` | 内部进程身份 |
| `from`、`to` | 动作开始时间的闭区间，Unix 纳秒 |
| `cursor` | 上一页返回的游标，筛选变化后重新开始查询 |
| `limit` | 页大小，默认 `250`，默认上限 `1000` |

#### 响应字段

| 字段 | 含义 |
|---|---|
| `actions` | 当前页动作热字段 |
| `links` | 动作的直接关联 |
| `context_actions` | 解释关联所需的辅助端点，可位于筛选范围外 |
| `next_cursor`、`has_more` | 继续查询的位置与状态 |
| `scanned` | 索引取回的全部候选数 |
| `links_truncated` | 关联达到读取上限时的截断标记 |
| `partial` | 当前结果的部分读取标记 |
| `root_command_before_window` | 窗口前的根命令快照，或 `null` |

#### 分页与读取

| 规则 | 行为 |
|---|---|
| 时间筛选 | 两端条件在索引查询中执行，已加载动作由浏览器裁剪到视窗 |
| 游标 | 记录最后扫描的存储时间和内部 action key，关联使用原始 action ID |
| 多类型查询 | 按各索引共同可确定的排序前缀推进游标 |
| 扫描上限 | 候选扫描上限至少覆盖去重后的类型数量，达到上限时可返回带游标的空页 |
| 动作内容 | 读取热字段，正文、冷属性和 evidence 由详情接口读取 |
| LLM Call 终态 | 最多额外读取两条响应关联与一条响应热记录 |
| HTTP 消息标题 | 使用已存储的请求方法与路径，或响应状态 |
| 运行中的 Trace | 刷新后更新范围和数据 |

#### 窗口前的根命令

| 条件或步骤 | 行为 |
|---|---|
| 触发条件 | 首页带 `from`，种类包含命令，进程筛选允许 Trace 根进程 |
| 查询身份 | Trace 的 `root_process_id` |
| 读取范围 | 按进程、类型与开始时间索引倒序读取窗口前最近的一条命令候选，再点读热字段 |
| 返回值 | 候选有效时返回原始命令快照，无有效候选或查询续页时返回 `null` |
| 查询计量 | 单独读取至多一条候选，普通 `limit`、`scanned` 和游标仅统计 `actions` 查询 |
| 生命周期 | 按原始观测的起止时间投影，同进程有序 exec 关闭前代命令 |

该字段补充 Trace 根进程的一条前驱命令。浏览器合并、裁剪和快照失效规则见[根命令快照](../../architecture/components/web-frontend/timeline.md#根命令快照)。

### Idle 区间

`GET /api/traces/{id}/idle-intervals` 查询已持久化的停滞区间（hang episode）。Local 与 Cluster 使用相同语义。

| 项目 | 约定 |
|---|---|
| `from`、`to` | Unix 纳秒闭区间，省略时从历史区间开始有限扫描 |
| `cursor`、`limit` | 继续游标与页大小，默认 `100`，默认上限 `500` |
| 响应 | `idle_intervals`、`next_cursor`、`has_more`、`scanned`、`axis_end_unix_nanos` |
| 区间字段 | `id`、`session_id`、`task_id`、`kind`、`start_time_unix_nanos`、可空的 `end_time_unix_nanos` |
| 排序 | 沿首次检测告警 ID 升序推进游标，筛选后的空页可带继续游标 |
| 开放区间终点 | 使用 `axis_end_unix_nanos`，终态 Trace 取最早终态时间，运行中取当前时间 |
| 浏览器加载 | 显式加载当前范围，逐页替换，筛选变化后重置游标 |

### 正文分页

| 接口或参数 | 约定 |
|---|---|
| `/api/traces/{id}/actions/{action_id}/content/llm-request` | 通过 `max_bytes` 读取 canonical JSON 前缀 |
| `/api/traces/{id}/actions/{action_id}/content/llm-request/node` | 通过 `pointer`、`offset`、`limit` 选择节点和分页 |
| 对象与数组 | 返回当前子项页 |
| 字符串 | 按 UTF-8 字节分页，`offset` 与 `next_offset` 定位，`value_bytes` 与 `returned_bytes` 表示总长度和本页长度 |
| 读取上限 | 沿 skeleton 和必要 block 导航，单个 block 超过剩余可读取字节数时返回请求错误 |
| 字符边界 | 单页字节上限须容纳下一个完整 UTF-8 字符，至少 4 字节可容纳任意单个字符 |

服务端字节与条目上限见 [Web 配置](../configuration/daemon.md#web-与-exportsnapshot)。完整内容表示见 [LLM Request 投影](../../architecture/components/llm-request-projection.md#故障边界)。

## 配置编辑

### 字段选择

| 项目 | 约定 |
|---|---|
| 策略来源 | `crates/core/config_export_filter/allowlist.toml`，与 `actrailctl init --filter` 共用 |
| `default = "whitelist"` | 选择 `whitelist` 列表，作为默认模式 |
| `default = "blacklist"` | 从完整默认文档中排除 `blacklist` 叶子字段 |
| 数组 | 整体选择，白名单与黑名单各自独立校验 |
| 生效时机 | 策略随编译嵌入，调整后重新构建 |
| 启动校验 | 使用模块默认配置模板校验路径，未知路径、列表内重复路径或非法模式导致启动失败 |
| 策略文件读取失败 | 构建时记录错误，使用配置编辑器时报错 |

普通 `actrailctl init` 导出完整配置，`--filter` 启用筛选。字段集合在 Web 启动后保持固定。

### 读取配置

`GET /api/config/editable` 要求 loopback 连接、匹配本机监听地址的 Host 和同源请求。

| 响应字段 | 含义 |
|---|---|
| `available`、`source` | 编辑可用状态与配置来源 |
| `fields` | 配置点分路径到字段值的映射 |
| `kinds` | 与 `fields` 键集相同，值为 `text`、`integer`、`boolean`、`string_array` 或 `toml` |

| 值与控件 | 约定 |
|---|---|
| 整数 | GET 以十进制字符串传输，POST 使用 JSON 整数 |
| 字符串数组 | 所属模块提供类型依据，空默认数组仍使用字符串数组控件 |
| 复杂数组、浮点数等 | 使用 `toml` 文本控件，GET 与 POST 均传字符串 |
| TOML 文本 | 内容为单个 TOML 值，例如 `[]` 或 `[{ name = "example" }]` |
| 值的来源 | 按控件类型依次选取可编码的文件原文、模块规范化配置和默认文档 |
| 禁用模块中的异型值 | 使用可编码的生效值或默认值，启动快照保留原文 |
| 省略的 SQLite 字段 | 从所属模块默认值读取 |

`GET /api/config/current` 的 `text` 返回启动时读取的配置文件原文，摘要来自启动生效配置。命令行覆盖作用于运行配置。编辑保存后，Web 重启时更新原文快照。

### 保存配置

`POST /api/config/editable` 沿用读取接口的连接条件，要求 `Content-Type: application/json`。请求为需修改路径和值的部分映射。

| 步骤 | 行为 |
|---|---|
| 1. 检查输入 | 校验字段选择策略及输入类型 |
| 2. 合并 | 在最新配置原文上逐键应用修改，保留其他值、注释和文档内容 |
| 3. 校验 | 所属模块解析并校验取值范围，例如后代观测深度允许 `-1`，事件块压缩等级允许 `-7..=22` |
| 4. 保存 | 原子落盘，相关进程按启动机制加载新配置 |

| 错误 | HTTP 状态 |
|---|---|
| 字段或输入错误 | `400` |
| 已有配置非法 | `409` |
| 读写失败 | `500` |

## 插件 Catalog

`GET /api/plugins/catalog`

| 字段 | 约定 |
|---|---|
| `directory` | 按配置顺序排列的已解析路径字符串数组，空列表返回 `[]` |
| `package_key` | 规范化的完整包目录路径，浏览器作为选择键原样传回 |
| `issue` | 包或根目录诊断 |
| `activation_ready` | 条目检查失败时为 `false` |

| 处理阶段 | 行为 |
|---|---|
| 扫描 | 复用 manifest 解析结果，按目录优先级选择插件 ID |
| 候选上限 | 全部候选共用 `max_packages`，检查通过后解析 manifest |
| 根目录诊断 | 每根最多一条，使用无损转义路径、根作用域与配置位置形成诊断键 |
| 请求加载 | 重新扫描，按 `package_key` 匹配当前选中的包，再向 daemon 发送控制请求 |
| 页面展示 | 目录数组显示为带引号的列表，路径中的逗号和空格保留为路径内容 |

发现顺序、重复 ID 与错误条目的完整规则见[插件管理](../../operations/plugins/manage.md#四种状态)。
