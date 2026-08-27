# OTEL exporter 规范

> 本文规定维护者实现或审查 `otel-jsonl` 与 `otel-http` 时必须保持的选择、交付和出境边界。

状态：已实现
范围：内置 `otel-jsonl` 与 `otel-http` 的 action 选择、交付和出境边界

## 共同选择语义

1. `[action_kinds]` 必须是 `{ canonical action kind -> boolean }` 映射，并包含保留字段
   `default`；未列出的 kind 使用 `default`。
2. 配置必须拒绝未知 kind、错误类型和 schema 不允许的额外字段。
3. 官方默认配置必须使用 `default = false`，并显式列出当前可配置 kind，避免新增 kind
   自动扩大遥测输出范围。
4. action 必须在进入 exporter 异步队列前过滤。
5. `file.tty_io` 是 recording 层的上游保护，不得出现在 exporter 配置或通过配置打开。
6. 插件未加载时不得读取业务配置、创建队列或初始化 sink。

## `otel-jsonl`

顶层 `exporter` 是唯一分派依据。`file` 与 `json_rpc_http` 的配置和初始化必须隔离；未
选中分支不得打开文件或发起网络请求。

### 文件 exporter

- `path` 必须是绝对路径。
- 路径无效、父目录无法创建或文件无法打开必须使实例加载失败，不得回退。
- 每个 OTLP JSON document 独占一行。
- `overwrite_enabled = true` 使用 truncate；否则只允许创建新文件。
- `flush_every_spans` 控制刷新频率，必须来自配置。

### JSON-RPC HTTP(S) exporter

每条 OTLP JSON document 作为一个 JSON-RPC 2.0 request 的 `params`。请求 ID 在实例内
单调递增；同一记录的重试必须保持 ID 不变。记录只有同时满足以下条件才确认交付：

1. HTTP 请求成功；
2. 响应体不超过配置上限；
3. 响应是 JSON-RPC 2.0 object；
4. 响应 ID 与请求一致；
5. 响应包含 `result` 且没有 `error`。

408、429、5xx、连接失败和超时可以在配置的 `max_attempts` 内重试；其他 HTTP 状态、
JSON-RPC error 或非法响应终止当前 worker。多次尝试形成 at-least-once 交付，对端必须
能够按 ID 去重。当前 HTTPS 支持公开 Web PKI 根证书；私有 CA、客户端证书和认证 header
不受支持。

## `otel-http`

1. 明文 endpoint 必须通过 `allow_insecure = true` 显式确认；TLS 参数只能用于 HTTPS。
2. mTLS client certificate 和 key 必须同时配置。
3. `action_kinds.default` 必须为 `false`；此插件的远端出境只能逐项允许。
4. `metadata-only` 不得发送命令行和 HTTP/LLM 内容；`full` 只能发送 daemon 已生成的属性，
   不得自行扩大本地内容生成或留存范围。
5. LLM request body 出境必须同时满足：本地可重建内容保留、正文属性导出、插件
   `attribute_mode = "full"`，并允许 `llm.request`。工具结果正文采用同样的分层授权。
6. endpoint 不可达是运行期下游故障，不得反向使插件加载或 daemon 失败；实例状态必须
   通过 error、retry、successful/dropped batch 等观测信号呈现。

## 热路径与失败

consumer 热路径只做一致性校验、action 选择、编码和有界 `try_send`。文件 I/O、DNS、
TCP、TLS、HTTP、响应解析、等待和重试必须在 exporter worker 执行。队列满只丢弃本地
实例的当前记录；最终交付失败只终止当前 exporter worker，其他记录链路继续运行。
