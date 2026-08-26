# Daemon 配置参考

> 本文说明 `actraild.conf` 中生命周期、存储、保留和本地接口字段的职责与默认值。

当前版本的完整字段、注释和值域由运行中版本内置模板定义。生成一份不影响系统配置的参考文件：

```bash
actrailctl init --output /tmp/actraild.reference.conf
```

始终用部署版本的 binary 生成模板。未知字段、无效值和缺少的必需关系会在启动时失败。

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
| `backend` | `sqlite` | 当前 operator storage backend |
| `[storage.sqlite].path` | `/var/lib/actrail/actrail.sqlite` | SQLite 主文件 |
| `busy_timeout_ms` | `5000` | 遇到暂时 lock 时的等待时间 |
| `cold_field_compression_min_bytes` | `64` | cold attribute 启用 zstd 的最小序列化大小；`0` 关闭 |
| `cold_field_zstd_level` | `3` | zstd level |

SQLite 使用 WAL 时，备份和恢复必须包含配套 WAL/SHM 状态或在 daemon 安全停止后取得一致副本。

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
