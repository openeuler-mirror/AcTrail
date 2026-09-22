# Daemon 配置参考

> 本文说明 `actraild.conf` 中生命周期、存储、保留和本地接口字段的职责与默认值。

当前版本的完整字段、注释和值域由运行中版本内置模板定义。生成一份不影响系统配置的参考文件：

```bash
actrailctl init --output /tmp/actraild.reference.conf
```

始终用部署版本的 binary 生成模板。未知字段、无效值和缺少的必需关系会在启动时失败。

## `[file_observation.collection]`

文件采集需求由下列字段指定，并与文件观测、语义投影和实际采集能力共同决定是否启用。原事件的 `raw_event_retention` 只控制留存，不启用采集需求。

| 字段 | 默认值 | 所需输出 |
| --- | --- | --- |
| `writable_open` | `true` | 可写、创建或截断打开的意图及结果；打开成功不代表实际发生写入 |
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

读取和写入在内核按实际文件对象累计，并按进程归属输出摘要。单个文件也能输出，不依赖扫描程序名单或跨文件数量阈值。

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `flush_interval_ms` | `100` | 用户态读取内核 I/O 累计状态的间隔，单位毫秒 |
| `max_entries` | `16384` | 内核文件 I/O 累计条目容量 |
| `object_max_entries` | `16384` | 内核文件对象身份条目容量 |

三个值均须大于零。`collection.read/write` 的 `observed`、`counts`、`bytes`、`errors` 独立控制所需事实、次数、字节和错误输出；全部关闭的方向不启动 I/O 汇总。`raw_event_retention` 不改变内核采集需求。`bulk_read` 仅消费文件摘要形成跨文件路径集合，不缓存完整 syscall 事件，不作候选回放。

TTY 同样使用此刷新间隔。`tty.operations` 默认且仅允许 `read/readv/write/writev`，其他操作在启动时拒绝。`read` 或 `readv` 任意一项选择读取方向，`write` 或 `writev` 任意一项选择写入方向；摘要不区分这两种 syscall。只请求 `errors` 时输出错误发生事实与 errno，不输出未请求的次数。

## `[control]`

| 字段 | 当前默认值 | 含义 |
| --- | --- | --- |
| `socket_path` | `/run/actrail/control.sock` | ctl 与 daemon 的本地 Unix socket |
| `socket_mode_octal` | `660` | socket 文件 mode |
| `pending_connection_max` | `256` | 同时等待处理的 control client 上限 |
| `active_trace_max` | `128` | 同时非终态 trace 的 admission 上限 |
| `pid_file` | `/run/actrail/actraild.pid` | `start/stop/status/restart` 使用的 PID 文件 |
| `log_path` | `/var/log/actrail/actraild.log` | 后台 daemon stdout/stderr 日志 |
| `diagnostic_log_level` | `info` | `off`、`info` 或 `debug` |

`[control.workload_diagnostics]` 默认关闭，`interval_ms = 1000`。`[control.finalization]` 控制 trace settle 与 shutdown drain；当前 `shutdown_drain_timeout_ms = 30000`。`[control.finalization.post_trace]` 为 post-trace broker、执行与 drain 提供彼此显式的容量和时间预算。

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

Web 默认监听 `127.0.0.1:18080`，request read timeout 为 `1000` ms。Web 还可能执行本地插件管理，不能直接暴露到不可信网络。

Snapshot 默认目录为 `/var/lib/actrail/export`，当前生成配置的 `payload_bytes_enabled` 与 `payload_text_enabled` 都为 `true`。它们控制 graph JSON 的 payload 内容，不等同于 LLM semantic body export 或实时 OTEL attribute mode。

## `[supervision]`

| 字段 | 当前默认值 | 含义 |
| --- | --- | --- |
| `startup_wait_ms` | `30000` | `start/restart` 等待 PID 与 control socket ready 的总预算 |
| `shutdown_wait_ms` | `5000` | supervising CLI 等待进程退出的预算 |
| `poll_interval_ms` | `100` | supervision 状态轮询间隔 |
