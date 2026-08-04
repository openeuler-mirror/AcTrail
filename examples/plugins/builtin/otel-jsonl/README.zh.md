# 内置 OTEL JSON 观测插件

类别：内置观测消费者。

这个示例使用 `runtime = "builtin"` 和 `id = "otel-jsonl"`。插件把 semantic
action 编码为 OTLP JSON，再交给配置选中的 exporter。当前 exporter 包括：

- `file`：逐行写入 JSONL 文件；
- `json_rpc_http`：通过 HTTP(S) 发送 JSON-RPC 2.0 请求。

两者是并列实现；后续 exporter 可以继续作为新的配置分支加入。release 安装器会把
这个完整插件包安装到
`${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}/otel-jsonl`，使它出现在 Web 的
**Plugin candidates** 中，但不会自动加载。

文件：

- `otel-jsonl.plugin.toml`：插件 manifest；文件名符合插件目录发现约定。
- `otel-jsonl.config.toml`：插件自己的 TOML 配置。
- `otel-jsonl.config.v1.schema.json`：`schema_ref` 指向的 JSON Schema。

插件配置中的 `exporter`、`queue_capacity` 和 `[action_kinds]` 是公共设置。
`default` 控制未显式列出的可导出 kind，其余 boolean 字段会由 Web 按 schema
显示为 checkbox。`file.tty_io` 由 recording 层在 exporter 之前过滤，不属于插件
配置。

选择文件输出：

```toml
exporter = "file"
queue_capacity = 1024

[file]
path = "/var/lib/actrail/export/live-spans.otlp.jsonl"
overwrite_enabled = true
flush_every_spans = 1
```

选择 JSON-RPC 2.0 over HTTP(S)：

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
```

每条 OTLP JSON 记录对应一个 JSON-RPC 请求，记录本身作为 `params`：

```json
{"jsonrpc":"2.0","id":1,"method":"otel.export","params":{"resourceSpans":[]}}
```

对端必须返回相同 `id`，并提供 `result`；JSON-RPC `error`、不匹配的 `id`、
无效响应或最终 HTTP 失败都会终止当前 exporter worker。`max_attempts` 只对
HTTP 408、429、5xx 及可恢复的连接/超时故障生效。大于 `1` 时，同一个请求 ID
可能被重复发送，对端应按 ID 去重。

所有网络请求和重试都发生在独立的有界交付线程。采集路径只执行非阻塞入队；
队列满或 exporter 失败时，记录只在这个插件内丢弃，不阻塞 recording 或其他插件。

加载示例：

```bash
target/release/actraild --config operator.conf plugin load \
  --manifest examples/plugins/builtin/otel-jsonl/otel-jsonl.plugin.toml \
  --plugin-config examples/plugins/builtin/otel-jsonl/otel-jsonl.config.toml \
  --instance dynamic.otel-jsonl
```

查看状态：

```bash
target/release/actraild --config operator.conf plugin status \
  --instance dynamic.otel-jsonl
```

也可以写入 `operator.conf`，让 daemon 启动时自动加载：

```toml
[plugins.startup]
enabled = true
failure_policy = "fail-fast"

[[plugins.startup.load]]
instance = "live-otel"
enabled = true
failure_policy = "continue"
manifest = "examples/plugins/builtin/otel-jsonl/otel-jsonl.plugin.toml"
plugin_config = "examples/plugins/builtin/otel-jsonl/otel-jsonl.config.toml"
host_grants = []
```

## 通过 Web 启用

1. 确认 `[plugins.discovery].directory` 指向安装器使用的插件根目录。
2. 打开 Web 的 **Plugins** 工作区并点击 **Refresh**。
3. 在 **Plugin candidates** 中找到 `otel-jsonl`。
4. 需要时先编辑安装目录中的 `otel-jsonl.config.toml`，再点击 **Configure & load**。
5. 使用实例 ID `live-otel` 或其他非空且未占用的名称完成加载。

加载后，该候选会进入 **Loaded plugin instances**。展开 **Configuration** 后，
可从 **Exporter** 下拉框选择文件或 JSON-RPC；Web 只显示当前 exporter 的配置
区域。修改须先通过 **Test configuration**，再执行 **Update configuration**。
这是现有的运行时配置更新：daemon 重启后仍会从插件包中的
`otel-jsonl.config.toml` 重新加载。

实例状态中的 `observed_records` 应随运行中的 semantic action 增长；
`dropped_records` 和 `last_error` 用于发现队列拥塞、文件写入或远端交付错误。
