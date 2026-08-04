# OTEL exporter 选择协议

`otel-jsonl` 面向需要实时转发 semantic action 的 AcTrail 运维者：插件只负责把
已选 action 编码为 OTLP JSON，并把交付方式隔离为可替换的 exporter。当前支持
本地 JSONL 文件和 JSON-RPC 2.0 over HTTP(S)；两者共享 action 筛选与有界队列，
但配置、初始化和失败状态彼此独立。

## 配置以 exporter 类型为唯一分派依据

顶层 `exporter` 是必填枚举。公共配置只包含 `queue_capacity` 和
`action_kinds`，实现专属参数分别放入 `[file]` 与 `[json_rpc_http]`：

```toml
exporter = "file"
queue_capacity = 1024

[file]
path = "/var/lib/actrail/export/live-spans.otlp.jsonl"
overwrite_enabled = true
flush_every_spans = 1

[json_rpc_http]
endpoint = "https://collector.example/v1/otel"
method = "otel.export"
connect_timeout_ms = 2000
request_timeout_ms = 5000
response_body_max_bytes = 65536
max_attempts = 1
retry_backoff_ms = 200
```

解析器必须拒绝未知 exporter、未知字段和缺失的活动分支。未选中的分支不参与
运行时初始化；修改未选中分支不得打开文件或发起网络请求。AcTrail 尚未达到
`v1.0`，旧版顶层 `path` 配置不保留兼容解析。

新增 exporter 时只能增加新的枚举分支、专属配置类型和 sink 实现，不得把协议
判断散落到 observation consumer 或 action 编码器中。

## 文件 exporter 保持 JSONL 语义

`file` 在插件实例加载时创建父目录并打开目标文件。`overwrite_enabled = true`
使用 truncate；否则只允许创建新文件。每个 OTLP JSON document 写为一行，
`flush_every_spans` 控制累计多少条后刷新 `BufWriter`。

路径必须是绝对路径。路径无效、目录不可创建或文件不可打开时，实例加载失败，
禁止回退到其他路径或 exporter。

## JSON-RPC exporter 每条记录获得明确确认

`json_rpc_http` 使用 HTTP POST，`Content-Type` 与 `Accept` 都是
`application/json`。每条 OTLP JSON document 是一个 JSON-RPC 2.0 request 的
`params`：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "otel.export",
  "params": {
    "resourceSpans": []
  }
}
```

`method` 由配置决定。请求 ID 在 exporter 实例内单调递增；重试同一条记录时保持
ID 不变。只有满足以下全部条件才确认一条记录 durable：

1. HTTP 请求成功；
2. 响应体不超过 `response_body_max_bytes`；
3. 响应是 JSON-RPC 2.0 object；
4. 响应 ID 与请求一致；
5. 响应包含 `result` 且不包含 `error` 成员。

HTTP 408、429、5xx、连接失败和超时可以在 `max_attempts` 范围内重试，每次间隔
`retry_backoff_ms`。其他 HTTP 状态、JSON-RPC error 或无效响应立即终止当前
exporter worker。`max_attempts > 1` 是 at-least-once 交付：如果对端已处理请求但
响应丢失，可能收到相同 ID 的重复请求，因此对端应该按 ID 去重。

HTTP(S) 客户端复用连接池；HTTPS 使用 rustls 和公开 Web PKI 根证书。当前协议
不配置私有 CA、客户端证书或请求认证头。

## 下游故障只影响当前 exporter

consumer 的热路径只完成 action 选择、OTLP JSON 编码和有界队列
`try_send`。文件 I/O、DNS、TCP、TLS、HTTP、重试等待和响应解析只在 exporter
worker 中执行。

队列满时当前插件本地丢弃记录，不阻塞 recording。最终交付失败会终止当前
exporter worker；后续记录快速报告 drop，daemon、trace recording 和其他插件继续
运行。通过 Web 更新配置或重新加载实例会构造新的 worker。

所有 timeout、响应上限、尝试次数和退避时间都必须来自插件配置，禁止无限重试。

## Web 只展示当前分支

JSON Schema 使用 `if/then/else` 根据 `exporter` 选择活动对象。通用 Web renderer
显示 exporter 下拉框，只展示当前分支的字段，并保留隐藏分支的草稿值，便于用户
来回切换而不丢配置。

配置更新沿用现有 runtime-config 生命周期：用户必须先执行 schema 校验，再更新
实例。更新会重建 builtin consumer，但不会写回插件包中的
`otel-jsonl.config.toml`；daemon 重启后重新读取磁盘配置。

预留的 `network-egress` grant 不属于本 exporter 协议，本设计不修改其声明、解析
或授权行为。

## 验收路径

真实 Agent 端到端用例必须从刷新后的官方默认配置加载插件，再通过 Web API：

1. 选择 `file`，验证所选 action kind 出现在目标 JSONL；
2. 选择 `json_rpc_http`，由真实本地 HTTP receiver 校验 method、ID、params 和响应；
3. 注入 HTTP 503 和响应读取超时，验证相同请求 ID 重试成功且插件保持 active；
4. 确认两种 exporter 都不输出未启用的 action kind，且 recording 正常完成。
