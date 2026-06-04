# Full Monitor 验证说明

这个目录提供一份尽量打开采集能力的 AcTrail 验证配置，用于通过真实启动的 agent/CLI 程序检查采集链路。

## 文件

- `operator.conf`：full-monitor operator 配置。
- `/tmp/actrail-full-monitor/`：运行时产物目录，包括 socket、pid、SQLite、导出文件和 daemon 日志。

## 采集范围

这份配置会启用：

- eBPF 进程生命周期、网络传输、文件路径、mmap、pipe/FIFO、Unix socket 观测。
- TLS 明文 payload 采集，backend 为 `payload_tls_capture_backend = tls-sync`。
- socket 明文 payload 采集，backend 为 `payload_socket_capture_backend = bpf-copy-seccomp-fallback`。
- stdin/stdout/stderr payload 采集。
- HTTP/1.x、HTTP/2 frame/data、SSE preview 投影。
- process seccomp exec context 和 agent invocation 检测。
- 进程树和系统资源指标。
- payload bytes/text 导出，以及 live OTEL JSONL 导出。

这份配置不会启用 fanotify enforcement，因为 enforcement 会改变目标程序行为；这里的目标是验证采集。

## 敏感数据

此配置为了便于验证原始 payload，设置为：

```conf
payload_tls_redaction_policy = disabled
payload_stdio_redaction_policy = disabled
payload_socket_redaction_policy = disabled
```

这会把 API key、Authorization header 等敏感内容持久化到 `/tmp/actrail-full-monitor/actrail.sqlite`。如果要降低风险，改成：

```conf
payload_tls_redaction_policy = authorization-header
payload_stdio_redaction_policy = authorization-header
payload_socket_redaction_policy = authorization-header
```

## 构建

在仓库根目录执行：

```bash
cargo build --release -p daemon -p ctl -p view -p tls_probe_point_finder -p tls_payload_probe_sync
```

## 启动 Daemon

默认使用后台模式启动，daemon stdout/stderr 会写入配置中的 `log_path`：

```bash
mkdir -p /tmp/actrail-full-monitor
target/release/actraild --config docs/examples/08.full-monitor-validation/operator.conf start
```

查看状态：

```bash
target/release/actraild --config docs/examples/08.full-monitor-validation/operator.conf status
```

停止：

```bash
target/release/actraild --config docs/examples/08.full-monitor-validation/operator.conf stop
```

重启：

```bash
target/release/actraild --config docs/examples/08.full-monitor-validation/operator.conf restart
```

后台启动后，daemon stdout/stderr 路径为：

```text
/tmp/actrail-full-monitor/actraild.log
```

如果要临时前台排查 daemon 启动过程，再使用：

```bash
target/release/actraild --config docs/examples/08.full-monitor-validation/operator.conf run
```

## Daemon 诊断输出

默认配置为：

```conf
diagnostic_log_level = info
```

这个级别用于避免 daemon 按每个 payload segment 打印大量持久化调试行。常规验证时，daemon 日志里会保留粗粒度生命周期诊断，例如：

```text
[info] agent_launch started ...
[info] agent_launch closed ...
```

`actrailctl launch` 所在终端还会显示 trace 进入 Active 和目标程序自身输出。后台 `start` 模式下，daemon 诊断输出进入 `/tmp/actrail-full-monitor/actraild.log`。

如果要排查每个 payload segment 是否被持久化，可以临时改成：

```conf
diagnostic_log_level = debug
```

debug 模式下，daemon 会高频输出每个 payload segment 的持久化诊断。这类日志表示 payload segment 已落库，但每段都会产生字符串格式化和 terminal I/O 开销。排查结束后建议改回 `info` 或 `off`。

如果使用前台 `run`，这些日志会直接出现在当前 terminal；如果使用默认的后台 `start`，这些日志会写入 `/tmp/actrail-full-monitor/actraild.log`。

## 可选探测点检查

launch 前可以先确认目标程序能生成完整 fast probe plan：

```bash
target/release/tls-probe-point-finder fast --provider auto --source auto claude
target/release/tls-probe-point-finder fast --provider auto --source auto opencode
target/release/tls-probe-point-finder fast --provider auto --source auto xiaoo
target/release/tls-probe-point-finder fast --provider auto --source auto /usr/bin/curl
```

期望结果是输出 `probe_plan`，并且 `points` 同时包含 outbound 和 inbound 采集点。OpenSSL/BoringSSL 通常会显示 `SSL_write`/`SSL_write_ex` 和 `SSL_read`/`SSL_read_ex`；rustls 应显示 rustls plaintext 采集点。

## 启动目标程序

验证 TLS payload 采集时，请使用 `actrailctl launch`。不要用 `track-add`，因为 `launch` 负责准备 `LD_PRELOAD`、sync event socket 环境变量，以及可选 seccomp 设置。

Claude 示例：

```bash
target/release/actrailctl --config docs/examples/08.full-monitor-validation/operator.conf launch \
  --name full-monitor-claude \
  -- claude -p "请直接回答：你好"
```

opencode 示例：

```bash
target/release/actrailctl --config docs/examples/08.full-monitor-validation/operator.conf launch \
  --name full-monitor-opencode \
  -- opencode run "请直接回答：“你好”"
```

xiaoo 示例：

```bash
target/release/actrailctl --config docs/examples/08.full-monitor-validation/operator.conf launch \
  --name full-monitor-xiaoo \
  -- xiaoo run -p "请直接回答：“你好”"
```

直接 HTTPS curl 验证：

```bash
target/release/actrailctl --config docs/examples/08.full-monitor-validation/operator.conf launch \
  --name full-monitor-curl-deepseek \
  -- curl https://api.deepseek.com/chat/completions \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${DEEPSEEK_API_KEY}" \
    -d '{"model":"deepseek-v4-flash","messages":[{"role":"user","content":"请直接回答“你好”"}],"thinking":{"type":"disabled"},"stream":false}'
```

## 查看结果

列出 trace：

```bash
target/release/actrailctl --config docs/examples/08.full-monitor-validation/operator.conf list-traces
```

下面命令里的 `trace-1` 需要替换成 `list-traces` 输出中的实际 trace id。

查看 payload 行：

```bash
target/release/actrailviewer --config docs/examples/08.full-monitor-validation/operator.conf payloads --trace-id trace-1 --head 80
```

期望看到的 payload source：

- `TlsUserSpace`：tls-sync 捕获到的 HTTPS 明文。
- `Syscall`：socket payload 旁证。
- `Stdio`：stdin/stdout/stderr 捕获结果。

查看某段 payload 正文：

```bash
target/release/actrailviewer --config docs/examples/08.full-monitor-validation/operator.conf payload --trace-id trace-1 --segment-id payload-1 --format text
```

查看应用层事件：

```bash
target/release/actrailviewer --config docs/examples/08.full-monitor-validation/operator.conf events --trace-id trace-1 --head 120
```

期望看到 HTTP 请求/响应事件；如果 provider 使用流式输出，也应看到 SSE 相关事件。

查看语义动作：

```bash
target/release/actrailviewer --config docs/examples/08.full-monitor-validation/operator.conf actions --trace-id trace-1 --head 120
```

查看进程树：

```bash
target/release/actrailviewer --config docs/examples/08.full-monitor-validation/operator.conf processes --trace-id trace-1 --head 120
```

查看诊断事件：

```bash
target/release/actrailviewer --config docs/examples/08.full-monitor-validation/operator.conf diagnostics --trace-id trace-1 --head 120
```

导出 graph 和 OTEL：

```bash
target/release/actrailviewer --config docs/examples/08.full-monitor-validation/operator.conf export-json --trace-id trace-1 --output /tmp/actrail-full-monitor/trace-1.json
target/release/actrailviewer --config docs/examples/08.full-monitor-validation/operator.conf export-otel --trace-id trace-1 --output /tmp/actrail-full-monitor/trace-1.otlp.json
```

## 停止和清理

后台 daemon 执行：

```bash
target/release/actraild --config docs/examples/08.full-monitor-validation/operator.conf stop
```

如果临时用了前台 `run`，也可以用 `Ctrl-C` 停止。

清理配置中的运行时产物：

```bash
python3 docs/examples/clean.py --example full-monitor
```

如果要硬清理这个验证目录的运行时数据：

```bash
rm -rf /tmp/actrail-full-monitor
```

## 关键配置

- `diagnostic_log_level = info`：默认只保留 agent launch started/closed 这类粗粒度诊断，不打印逐 payload segment 的 debug 诊断。
- `event_ring_buffer_max_bytes = 8388608`：高流量观测使用的 eBPF event ring 大小。
- `file_path_max_bytes = 255`：BPF 文件路径事件拷贝上限，不能超过当前编译 ABI 最大值。
- `payload_tls_capture_backend = tls-sync`：通过 preload hook 同步采集 TLS 明文。
- `payload_tls_max_operation_bytes = 4194304`：sync runtime 可接受的单次 TLS payload 操作上限。
- `payload_tls_ring_buffer_bytes = 8388608`：保留给非 sync backend 和诊断尺寸兼容。
- `payload_tls_retention_max_bytes_per_trace = 104857600`：每个 trace 的 TLS payload 保留预算。
- `payload_tls_sync_match_limit = 8`：finder fast 每个 pattern 最多检查的匹配数量。
- `payload_socket_capture_backend = bpf-copy-seccomp-fallback`：socket payload 使用 BPF copy，并对大操作使用 seccomp fallback。
- `payload_socket_http_sniff_max_bytes = 65536`：socket 明文 HTTP 嗅探预算。
- `application_http_sse_data_policy = preview`：持久化 SSE data preview。
- `application_http_sse_max_data_bytes = 65536`：单条 SSE data preview 的大小预算。
- `application_http2_max_frame_bytes = 65536`：HTTP/2 analyzer 接受的 frame 大小。
- `process_seccomp_max_args = 128`：exec argv 捕获数量上限。
- `process_seccomp_max_arg_bytes = 8192`：exec argv 内容捕获字节上限。
- `resource_metrics_interval_ms = 1000`：资源指标采样周期。
