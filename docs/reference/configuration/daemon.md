# Daemon 配置参考

> 本文说明 `actraild.conf` 中生命周期、存储、保留和本地接口字段的职责与默认值。

配置类型、默认值和值域由各配置所属模块定义。默认初始化导出所属模块提供的完整配置：

```bash
actrailctl init --output /tmp/actraild.reference.conf
```

使用 `actrailctl init --filter --output /tmp/actraild.delivery.conf` 显式启用导出筛选，默认项按 [配置选择策略](../../../crates/core/config_export_filter/allowlist.toml) 选择。`default = "whitelist"` 选择 `whitelist` 中的字段；`default = "blacklist"` 从所属模块完整默认文档中排除 `blacklist` 中的字段。初始模式为 `whitelist`，黑名单为空。两个列表独立生效，字段可同时存在于两个列表中；未选中的列表仍会校验。`default` 仅选择名单模式，普通 `init` 继续导出完整配置。

省略项由所属模块补齐默认值；完整导出和筛选导出均保留 `--mode` 和 `--patch` 的显式设置，包括名单隐藏的字段和值。黑名单控制默认项选择，显式模式与补丁仍可写入其中的字段。Web 可编辑项使用同一份选择策略。

Web 只读启动快照使用启动时读取的配置文件原文：普通 `init` 展示完整默认配置项，`init --filter` 展示生成文件中的筛选后配置项，手写隐藏项也随文件内容展示。Web 命令行覆盖作用于运行配置，摘要根据启动时的生效配置生成。保存配置会更新文件，快照和摘要在重启 Web 后重新生成。

策略文件随 `actrailctl` 和 `actrailweb` 编译，调整后重新构建并部署两个程序。导出筛选默认关闭，策略文件存在、缺失或无效时均可正常导出完整配置。源码中的策略文件缺失时仍可构建；启用筛选或初始化 Web 编辑器时，缺失策略文件会明确报错。

两个名单采用点分叶子路径，数组整体选择；字段必须在所属模块提供的默认文档中具有值。Web 通用控件支持字符串、整数、布尔和字符串数组，复杂数组等值使用 TOML 文本控件，保存时仍由所属模块校验。带单位等字段使用所属领域的转换规则。所属模块提供字符串数组类型查询，默认空数组也可直接加入名单并编辑。默认省略的可选字段仍需定义导出与未设置值的编辑语义。使用策略时，同一列表内路径重复、格式错误、表选择和缺少模板值会明确报错。

始终用部署版本的 binary 生成配置。手写的隐藏字段仍参与完整配置加载与校验，Web 保存可见字段时保留这些字段及其值。未知字段、无效值和缺少的必需关系会在启动时失败。

## `[file_observation.collection]`

文件采集需求由下列字段指定，并与文件观测、语义投影和实际采集能力共同决定是否启用。原事件的 `raw_event_retention` 只控制留存，不启用采集需求。

| 字段 | 默认值 | 所需输出 |
| --- | --- | --- |
| `writable_open` | `true` | 可写、创建或截断打开的意图及打开结果 |
| `path_mutations` | `true` | rename、unlink、mkdir、rmdir、truncate 等路径修改及结果 |
| `fd_mutations` | `false` | ftruncate、共享可写 mmap 等需要 FD 路径关联的修改及结果 |

`[file_observation.collection.read]` 和 `[file_observation.collection.write]` 分别声明读取和写入需求：

| 字段 | 默认值 | 所需输出 |
| --- | --- | --- |
| `observed` | `false` | 文件上发生相应 I/O 的事实 |
| `counts` | `false` | I/O 次数摘要 |
| `bytes` | `false` | 实际读取或写入的字节数摘要 |
| `errors` | `false` | 相应 I/O 的错误信息 |

四项表达下游需求；任意一项启用才需要相应 I/O 采集。当前读取和写入分别按四项的合并需求启停。

P 模板采用上述默认值。C 模板显式开启 FD 修改及读取、写入需求，用于文件动作与统计展示。即使普通文件 I/O 全关，路径修改所需的目录 FD 与 cwd 上下文仍按需维护。

`file_observation.tty.enabled` 和 `file_observation.enumerate.enabled` 默认及 P 模板均为 `false`，C 模板显式开启。目录枚举消费目录打开与关闭事实，独立于普通文件读取；TTY 的读取、写入分析需要显式启用对应方向的 collection 需求。文件观测与 TTY 启用时，`tty.operations` 包含 `read/readv` 而读取需求全关，或包含 `write/writev` 而写入需求全关，均在启动时拒绝；不会自动开启 I/O。

## `[file_observation.summary]`

读取和写入在内核按实际文件对象累计，并按进程归属输出摘要。摘要适用于任意程序的单个或多个文件。

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `flush_interval_ms` | `100` | 用户态读取内核 I/O 累计状态的间隔，单位毫秒 |
| `max_entries` | `16384` | 内核文件 I/O 累计条目容量 |
| `object_max_entries` | `16384` | 内核文件对象身份条目容量 |

三个值均须大于零。`collection.read/write` 的 `observed`、`counts`、`bytes`、`errors` 独立控制所需事实、次数、字节和错误输出；全部关闭的方向不启动 I/O 汇总。`raw_event_retention` 控制原始事件留存，采集需求由 `collection.read/write` 决定。`bulk_read` 消费文件摘要形成跨文件路径集合。

TTY 同样使用此刷新间隔。`tty.operations` 默认且仅允许 `read/readv/write/writev`，其他操作在启动时拒绝。`read` 或 `readv` 任意一项选择读取方向，`write` 或 `writev` 任意一项选择写入方向；摘要不区分这两种 syscall。只请求 `errors` 时输出错误发生事实与 errno，不输出未请求的次数。

## `[control]`

| 字段 | 当前默认值 | 含义 |
| --- | --- | --- |
| `socket_path` | `/run/actrail/control.sock` | ctl 与 daemon 的本地 Unix socket |
| `socket_mode_octal` | `660` | socket 文件 mode |
| `pending_connection_max` | `256` | 同时跟踪的 control 连接数上限 |
| `accept_retry_delay_ms` | `10` | 接收连接失败后重试的间隔，正整数毫秒 |
| `active_trace_max` | `128` | 同时非终态 trace 的 admission 上限 |
| `pid_file` | `/run/actrail/actraild.pid` | `start/stop/status/restart` 使用的 PID 文件 |
| `log_path` | `/var/log/actrail/actraild.log` | 后台 daemon stdout/stderr 日志 |
| `diagnostic_log_level` | `info` | `off`、`info` 或 `debug` |

daemon 的 tracing 日志使用 `YYYY-MM-DDTHH:MM:SS+08:00 [info] ...` 格式，固定为北京时间 UTC+8，独立于进程 `TZ` 或系统时区。控制 socket 就绪后记录 `daemon started`，正常关闭并完成清理后记录 `daemon stopped`。已有 `daemon listening ...` stdout 就绪信息保留；后台 stdout/stderr 写入 `log_path`，前台 tracing 写入 stderr。

启动时校验时间能否转换；运行期间转换失败时以 `time_unavailable` 标记保留事件内容。后台启动失败会给出配置的日志位置，并尝试附上本次启动的最后一行输出。读取范围由 `ACTRAIL_DAEMON_STARTUP_LOG_MAX_BYTES` 控制，默认 `16384`，必须为正整数；只在启动失败时读取此范围，日志轮换或读取失败会明确报告，已有错误原因仍保留。

日志预算在 start/restart 入口验证，restart 验证成功后再停止当前进程。非法值直接报告变量、输入值及修正或恢复默认值的方法。前台失败先记录操作指引，再记录原始原因；后台管理命令回传原因并附一次自身指引。

接收连接遇到 `EMFILE`、`ENFILE` 等暂时性错误时，daemon 暂停监听新连接，按
`accept_retry_delay_ms` 重试；已有控制连接、采集事件和后台任务继续处理。连续失败仅在
首次记录告警，恢复接入时记录恢复日志。单个连接初始化失败只关闭该连接。

`pending_connection_max` 与 `active_trace_max` 都须大于零；活动 trace 达到上限时，
新增请求返回 `active_trace_limit`。这两个容量限制分别作用于已接受连接和 trace 准入。
控制连接在请求或回复尚未完成时持续占用名额，回复完成、连接关闭或失败后释放；达到
上限时关闭刚接受的新连接。
进程文件描述符预算还需覆盖采集器、数据库、日志及各 trace 的实际开销；部署时通过
systemd `LimitNOFILE` 或启动环境的 `ulimit -n` 设置。资源不足期间新请求可能超时，
资源释放后恢复接入。
文件描述符不足也可能使资源采样等依赖文件访问的功能局部降级；相关诊断和 trace
健康状态会保留实际采集缺口。

插件的 `candidate_max_count`、`trace_state_max_count` 分别限制每 trace 候选路径和
每插件实例保存的 trace 状态，随附配置值为 `4096`、`256`，见
[file-leakage 插件配置](../../../examples/plugins/wit-component/file-leakage/README.zh.md)。

`[control.workload_diagnostics]` 默认关闭，`interval_ms = 1000`。`[control.finalization]` 控制 trace settle 与 shutdown drain；当前 `shutdown_drain_timeout_ms = 30000`。`[control.finalization.post_trace]` 为 post-trace broker、执行与 drain 提供彼此显式的容量和时间预算。

工作负载诊断日志中的 `background_ready` 统计轮询返回后后台截止时间已到的轮次，
可与 `event_source_ready` 在同轮增加；单独的接收重试唤醒不会增加该计数。跨实现比较
趋势时需注意，早期实现仅在 `ppoll` 的有限超时到期时增加 `background_ready`。

## `[checkpoint.runtime]`

checkpoint 启用开关、记录库和 backend 列表仍由 `[checkpoint]` 配置。运行参数如下，时间单位均为毫秒：

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `busy_timeout_ms` | `5000` | daemon 和 Web 的 checkpoint 只读连接等待 SQLite 写锁的预算 |
| `stop_timeout_ms` | `5000` | 恢复前等待一批进程线程组暂停的预算 |
| `stop_poll_interval_ms` | `10` | 检查线程组暂停状态的间隔 |
| `shutdown_drain_timeout_ms` | `10000` | daemon 停止时等待 checkpoint 工作完成的总预算 |
| `idle_poll_interval_ms` | `10` | 停机等待期间检查工作是否完成的间隔 |

各项须大于零。省略字段使用默认值；`actrailctl init` 生成的模板包含这些参数。操作步骤见 [checkpoint](../../operations/checkpoint.md)。

## `[storage]`

| 字段 | 当前默认值 | 含义 |
| --- | --- | --- |
| `backend` | `sqlite` | `sqlite` 保存观测历史；`noop` 丢弃观测写入、历史查询为空，不创建主 SQLite |
| `[storage.sqlite].path` | `/var/lib/actrail/actrail.sqlite` | SQLite 主文件 |
| `busy_timeout_ms` | `5000` | 遇到暂时 lock 时的等待时间 |
| `cold_field_compression_min_bytes` | `64` | cold attribute 启用 zstd 的最小序列化大小；`0` 关闭 |
| `cold_field_zstd_level` | `3` | zstd level |
| `event_payload_dictionary_cache_bytes` | `33554432` | 单次 daemon 会话内的事件 payload 精确去重缓存近似上限；不扫描历史库，`0` 禁用并全部 inline |
| `event_path_dictionary_cache_bytes` | `8388608` | 单 writer、按 trace 的统一 path interner 缓存上限；event 与 semantic path 共用 `path_id`，容量耗尽只降低缓存命中率，不改变持久化格式 |
| `event_record_layout` | `rows` | 原始事件物理布局；`rows` 保持逐行存储，`blocks` 启用无损事件块，两种布局读取时可共存 |
| `event_record_block_max_events` | `256` | `blocks` 模式单块最大逻辑事件数，范围 `1..=65536` |
| `event_record_block_max_uncompressed_bytes` | `1048576` | `blocks` 模式单块及每 trace 未满尾块的编码前字节上限；范围 `1..=67108864`，单事件超过上限时该次写入 fail-local |
| `event_record_block_zstd_level` | `3` | `blocks` 模式 zstd level，范围 `-7..=22` |

SQLite 使用 WAL 时，备份和恢复必须包含配套 WAL/SHM 状态或在 daemon 安全停止后取得一致副本。

事件块仅改变 SQLite 物理布局：`event_id`、时间、顺序、进程、payload、policy 与 semantic evidence 引用保持完整。未满块以每 trace 有界的 pending frame 持久化，跨事务继续填充；达到事件数/字节边界或 trace 进入 terminal 状态时，在同一事务中原子压块。提交后的 pending event 可立即查询，重启后也能继续填充。未知 codec 或越过安全边界的损坏记录会使该次 trace 读取失败，不会让 daemon 进程崩溃。`blocks` 直接压缩完整 payload，因此 `event_payload_dictionary_cache_bytes` 只作用于 `rows`。默认继续使用 `rows`，便于按部署显式启用并测量写入与查询成本。

`[storage.retention]` 当前默认启用：`max_trace_age = "7d"`、`sweep_interval = "1m"`、`min_terminal_age = "30s"`、每轮最多 `10` 个 trace，并保护 tag `retain` 和 `pinned`。生产环境应按调查保留期、磁盘容量和合规要求调整。

## `[web]` 与 `[export.snapshot]`

Web 提供本地分析与插件管理，部署范围为可信网络。

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `web.listen_addr` | `"127.0.0.1:18080"` | HTTP 监听地址 |
| `web.request_read_timeout_ms` | `"1000"` | 请求读取超时，单位毫秒，使用字符串配置。`"disabled"` 关闭超时 |

### Checkpoint 页面

以下字段属于 `[web.checkpoints]`，单位为毫秒。

| 字段 | 默认值 | 取值范围 | 含义 |
| --- | --- | --- | --- |
| `status_timeout_ms` | `500` | 大于 `0` | 查询 daemon 中 checkpoint 暂停和执行状态的超时 |
| `poll_interval_ms` | `1500` | `1..=2147483647` | 有运行中 Trace 或未完成工作时，页面自动刷新的间隔 |

| 条件 | 行为 |
| --- | --- |
| 修改配置 | 重启 `actrailweb` 后生效 |
| 状态查询超时 | 继续显示历史，运行状态显示为未知 |
| 首次请求失败，尚未取得刷新配置 | 按默认 `1500` 毫秒重试 |

### 查询环境变量

`actrailweb` 在启动时读取以下环境变量。

| 查询类型 | 配置表 | 接口约定 |
| --- | --- | --- |
| Trace 列表 | [列表限制](#trace-列表环境变量) | [列表分页](../protocols/web-api.md#trace-列表) |
| 时间线 | [时间线限制](#时间线环境变量) | [时间线接口](../protocols/web-api.md#时间线) |
| Idle 区间 | [Idle 限制](#idle-环境变量) | [Idle 查询](../protocols/web-api.md#idle-区间) |
| 正文 | [正文限制](#正文环境变量) | [正文分页](../protocols/web-api.md#正文分页) |

#### Trace 列表环境变量

| 环境变量 | 默认值 | 含义 |
| --- | ---: | --- |
| `ACTRAIL_WEB_TRACES_DEFAULT_LIMIT` | `50` | 默认每页条目数 |
| `ACTRAIL_WEB_TRACES_MAX_LIMIT` | `250` | 客户端可请求的每页条目数上限 |

#### 时间线环境变量

| 环境变量 | 默认值 | 含义 |
| --- | ---: | --- |
| `ACTRAIL_WEB_TIMELINE_DEFAULT_LIMIT` | `250` | 默认每页动作数 |
| `ACTRAIL_WEB_TIMELINE_MAX_LIMIT` | `1000` | 客户端可请求的每页动作数上限 |
| `ACTRAIL_WEB_TIMELINE_CANDIDATES` | `2000` | 单页候选扫描上限 |
| `ACTRAIL_WEB_TIMELINE_LINKS` | `2000` | 单页直接关联读取上限 |

#### Idle 环境变量

| 环境变量 | 默认值 | 含义 |
| --- | ---: | --- |
| `ACTRAIL_WEB_IDLE_DEFAULT_LIMIT` | `100` | 默认每页区间数 |
| `ACTRAIL_WEB_IDLE_MAX_LIMIT` | `500` | 客户端可请求的每页区间数上限 |
| `ACTRAIL_WEB_IDLE_CANDIDATES` | `1000` | 单页候选扫描上限 |

#### 正文环境变量

| 环境变量 | 默认值 | 含义 |
| --- | ---: | --- |
| `ACTRAIL_WEB_CONTENT_MAX_BYTES` | `16777216` | LLM request、MCP content 与普通 HTTP 正文的 `max_bytes` 上限 |
| `ACTRAIL_WEB_CONTENT_NODE_MAX_ITEMS` | `250` | 节点页的子项数量上限 |
| `ACTRAIL_WEB_CONTENT_NODE_PREVIEW_BYTES` | `65536` | 字符串叶子与普通 HTTP 正文默认单页 UTF-8 字节上限 |
| `ACTRAIL_WEB_CONTENT_NODE_BLOCK_BYTES` | `8388608` | 请求内 block 累计读取字节上限，同时限制普通 HTTP 正文的单次解码量 |

#### 校验与读取规则

| 范围 | 启动校验条件 |
| --- | --- |
| 全部环境变量 | 正整数，且小于 `i64::MAX` |
| Trace 列表 | `DEFAULT_LIMIT <= MAX_LIMIT` |
| 时间线、Idle | 各自满足 `DEFAULT_LIMIT <= MAX_LIMIT <= CANDIDATES` |

校验失败时，Web 启动失败。

| 查询 | 读取行为 |
| --- | --- |
| 本地与 Cluster Trace 列表 | 按 `page` 和 `limit` 查询，返回过滤后的准确总数与总页数 |
| Trace 计数与跳页 | 按查询需要扫描元数据，页大小上限只限制返回条目数 |
| 普通 HTTP 正文 | 每页重新读取并解码相关正文，读取与解码受 block 字节上限约束 |
| 正文读取超出上限 | 返回当前请求错误 |

### Snapshot 导出

| 配置项 | 默认值 | 含义 |
| --- | --- | --- |
| `export.snapshot.directory` | `"/var/lib/actrail/export"` | Snapshot 输出位置 |
| `export.snapshot.payload_bytes_enabled` | `true` | 在 graph JSON 中导出 payload 字节内容 |
| `export.snapshot.payload_text_enabled` | `true` | 在 graph JSON 中导出 payload 文本内容 |

## `[supervision]`

| 字段 | 当前默认值 | 含义 |
| --- | --- | --- |
| `startup_wait_ms` | `30000` | `start/restart` 等待 PID 与 control socket ready 的总预算 |
| `shutdown_wait_ms` | `160100` | supervising CLI 等待进程退出的预算；不得小于各项停机排空预算之和，启用 checkpoint 时包括 `checkpoint.runtime.shutdown_drain_timeout_ms` |
| `poll_interval_ms` | `100` | supervision 状态轮询间隔 |
