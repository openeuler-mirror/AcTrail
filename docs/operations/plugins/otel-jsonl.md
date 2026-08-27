# 启用 otel-jsonl

> 本文指导插件管理员将选定的 semantic action 写入本地 OTLP JSONL，或交付给 JSON-RPC HTTP(S) 接收端。

`otel-jsonl` 是内置异步观测插件，把选中的 semantic action（AcTrail 归一化后的行为记录）编码为 OTLP JSON。
它支持本地 JSONL 文件和 JSON-RPC 2.0 over HTTP(S) 两种 exporter，不需要 host grant。
插件安装、发现和加载的边界见 [管理插件](manage.md)。

## 选择 exporter

文件输出：

```toml
exporter = "file"
queue_capacity = 1024

[file]
path = "/var/lib/actrail/export/live-spans.otlp.jsonl"
overwrite_enabled = true
flush_every_spans = 1

[action_kinds]
default = false
"process.exec" = true
"process.exit" = true
"llm.request" = true
"llm.response" = true
```

`path` 必须是绝对路径。加载时创建父目录并打开文件；无法创建或打开会使加载失败，
不会回退到其他路径。`overwrite_enabled = true` 会截断目标文件，否则只允许创建新文件。

JSON-RPC HTTP(S) 输出：

```toml
exporter = "json_rpc_http"
queue_capacity = 1024

[json_rpc_http]
endpoint = "https://collector.example/v1/otel"
method = "otel.export"
connect_timeout_ms = 2000
request_timeout_ms = 5000
response_body_max_bytes = 65536
max_attempts = 1
retry_backoff_ms = 200

[action_kinds]
default = false
"llm.request" = true
"llm.response" = true
```

当前 HTTPS 使用公开 Web PKI 根证书；该 exporter 不提供私有 CA、客户端证书或认证 header
配置。所有 timeout、响应上限、尝试次数和退避都来自插件配置。

`[action_kinds]` 的 key 必须是带引号的 canonical action kind。未知、拼错或非布尔值会
使加载失败。官方默认使用 `default = false`，避免新 action kind 自动扩大出境范围。
`file.tty_io` 在上游永久过滤，不能通过插件配置打开。

## 加载和查看状态

```bash
sudo target/release/actraild --config operator.conf plugin load \
  --manifest /absolute/path/otel-jsonl.plugin.toml \
  --plugin-config /absolute/path/otel-jsonl.config.toml \
  --instance live-otel

sudo target/release/actraild --config operator.conf plugin status --instance live-otel
```

配置只初始化选中的 exporter 分支。Web 更新运行配置会重建 builtin consumer，但不会
写回插件包中的配置；daemon 重启后仍从磁盘配置读取。

## 运行期故障

action 选择和入队发生在热路径，文件 I/O、网络、TLS、HTTP 和重试在独立 worker 中。
队列满时只丢弃当前插件的记录，不阻塞 recording；最终交付失败会停止当前 exporter
worker，daemon、trace recording 和其他插件继续运行。管理员修复配置后需要更新或重新加载实例。

JSON-RPC 仅在 HTTP 成功、响应大小合规、响应为 JSON-RPC 2.0 object、ID 匹配且包含
`result` 而没有 `error` 时确认记录。408、429、5xx、连接失败和超时可在配置次数内重试；
重试复用同一个请求 ID。`max_attempts > 1` 是 at-least-once 交付，对端应按 ID 去重。
