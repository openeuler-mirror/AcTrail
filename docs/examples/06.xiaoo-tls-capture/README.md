# xiaoO TLS Payload Capture

这个示例验证真实 xiaoO 进程的 LLM request/response payload 采集。xiaoO 项目地址是 <https://gitcode.com/openeuler/xiaoO>。

目标是让测试人员在不了解 AcTrail 内部实现的情况下完成一轮验证：用 `actrailctl launch` 启动 `xiaoo`，由 `tls-sync` 在 TLS 明文边界上报 payload，再通过 `actrailviewer` 查看 payload、HTTP/SSE events 和 `llm.request`/`llm.response` semantic action。

如果 xiaoO 的 provider 路径是 HTTPS/TLS，期望 payload 来源为 `TlsUserSpace`。如果 xiaoO 被配置成 plain HTTP provider route，payload 来源可以是 `Syscall/socket-syscall`。如果只看到 HTTP proxy `CONNECT`，说明 socket 只能证明代理隧道，不能解密 request body，需要可用的 TLS plaintext probe plan 或 plain HTTP provider route。

## 文件

| 文件 | 用途 |
| --- | --- |
| `operator.conf` | xiaoO payload capture operator config，启用 `tls-sync`、socket payload、stdio payload、HTTP/HTTP2/SSE analyzer 和 payload export。 |

## 前置条件

- 在 Linux/WSL root shell 中运行。
- 先完成 release 构建：`cargo build --release`。
- `xiaoo` 在 `PATH` 中；`which xiaoo` 应打印本次要启动的可执行文件。
- xiaoO 已配置好 provider 凭据，并且可以直接运行一次普通请求。
- 如果目标是 HTTPS/TLS payload，先确认 finder fast 能给 xiaoO 返回完整 outbound/inbound plan：

```bash
target/release/tls-probe-point-finder fast --provider rustls --source auto xiaoo
```

期望输出中 `probe_plan.provider = rustls`，并且 `points` 至少包含：

```text
rustls_buffer_plaintext
rustls_take_received_plaintext
```

`operator.conf` 里的 TLS source/resolver 字段保持为可通过配置校验的 OpenSSL shared-library 组合；`tls-sync` 的实际 attach plan 由 `actrailctl launch` 对命令中的 `xiaoo` 调 finder fast 生成。

## 运行

清理上一次运行产物：

```bash
python3 docs/examples/clean.py --example xiaoo-tls
```

启动 daemon：

```bash
target/release/actraild --config docs/examples/06.xiaoo-tls-capture/operator.conf start
target/release/actrailctl --config docs/examples/06.xiaoo-tls-capture/operator.conf doctor
```

通过 `actrailctl launch` 启动 xiaoO：

```bash
target/release/actrailctl --config docs/examples/06.xiaoo-tls-capture/operator.conf launch \
  --name xiaoo-tls-payload \
  -- \
  xiaoo run --no-tools --max-turns 1 --prompt "请直接回答：你好"
```

记录输出中的 trace id，例如 `trace-1`。如果被测 xiaoO CLI 使用 `-p` 传入 prompt，可将最后一行替换成：

```bash
xiaoo run -p "请直接回答：你好"
```

## 查看结果

列出 trace：

```bash
target/release/actrailctl --config docs/examples/06.xiaoo-tls-capture/operator.conf list-traces
```

查看 payload 列表：

```bash
target/release/actrailviewer --config docs/examples/06.xiaoo-tls-capture/operator.conf payloads --trace-id trace-1 --head 80
```

通过条件：

- HTTPS/TLS 路径应出现 `SOURCE=TlsUserSpace`、`LIBRARY=rustls` 的 complete outbound payload。
- plain HTTP 路径可以出现 `SOURCE=Syscall`、`LIBRARY=socket-syscall` 的 complete outbound payload。
- 仅出现 `CONNECT <host>:443` 不算 request body 捕获成功。

查看某段 payload 正文：

```bash
target/release/actrailviewer --config docs/examples/06.xiaoo-tls-capture/operator.conf payload --trace-id trace-1 --segment-id payload-1 --format text
```

查看应用层 events：

```bash
target/release/actrailviewer --config docs/examples/06.xiaoo-tls-capture/operator.conf events --trace-id trace-1 --head 120
```

期望能看到 HTTP request/response。流式 provider 还可能出现 SSE events；SSE 分片属于服务端实际发送内容，不要求每条都有完整自然语言文本。

查看 semantic actions：

```bash
target/release/actrailviewer --config docs/examples/06.xiaoo-tls-capture/operator.conf actions --trace-id trace-1 --head 120
```

期望出现 `llm.request`，状态为 success/complete。配置保留 inbound response payload 时，还应出现 `llm.response`；如果只看到 response transport/event 但没有 `llm.response`，优先排查 inbound TLS/plain HTTP payload 是否被完整保留、HTTP/SSE analyzer 是否启用，以及 provider 是否实际返回 JSON/SSE body。流式响应的多段 SSE 会聚合到同一条 `llm.response`，其 evidence 数量可能大于 1。

导出 graph 和 OTEL：

```bash
target/release/actrailviewer --config docs/examples/06.xiaoo-tls-capture/operator.conf export-json --trace-id trace-1 --output /tmp/actrail-xiaoo-tls.json
target/release/actrailviewer --config docs/examples/06.xiaoo-tls-capture/operator.conf export-otel --trace-id trace-1 --output /tmp/actrail-xiaoo-tls.otlp.json
```

## 诊断

默认配置使用：

```conf
diagnostic_log_level = info
```

这个级别不会按每个 payload segment 打印 debug 日志。只有排查落库或 probe plan 问题时，才临时改成 `debug`。

如果 `tls-probe-point-finder fast --provider rustls --source auto xiaoo` 失败：

- 确认 `which xiaoo` 指向的是预期 binary。
- 确认目标架构有 rustls static pattern 支持；x86_64 stripped xiaoO 需要有可用的 rustls pattern 支持。
- 如果 provider 路径是 HTTP CONNECT 且没有 TLS plaintext plan，socket payload 不能解密 request body。

## 停止和清理

```bash
target/release/actraild --config docs/examples/06.xiaoo-tls-capture/operator.conf stop
python3 docs/examples/clean.py --example xiaoo-tls
```
