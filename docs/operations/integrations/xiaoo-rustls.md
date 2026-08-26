# 捕获 xiaoO rustls LLM 请求

> 本文说明如何在不修改 xiaoO 源码的情况下捕获 rustls HTTPS 明文并查看 `llm.request`。

该路径需要 `actrailctl launch`。它不支持用 `track-add` 事后安装 TLS sync runtime。

## 前置条件

- AcTrail 默认配置已生成，daemon 尚未启动或可安全重启；
- `target/release/tls-probe-point-finder` 和 TLS sync runtime library 已构建；
- 生产实际使用的 `xiaoo` 位于 `PATH`；
- xiaoO 自身的 provider/model/API-key 环境已经配置，且 API key 未写入 argv。
- 运行 `actrailctl` 的用户有权访问 operator config 指定的 control socket。

命令从 AcTrail 仓库根目录运行，因为 probe point finder 与 control binary 使用 `target/release/` 中本次构建的产物。

## 1. 验证实际二进制的 auto plan

```bash
export XIAOO_BINARY=/absolute/path/to/xiaoo
test -x "$XIAOO_BINARY"

./target/release/tls-probe-point-finder fast \
  --provider rustls \
  --source auto \
  --match-limit 8 \
  "$XIAOO_BINARY"
```

输出必须包含 `provider = rustls`、`rustls_buffer_plaintext` 和 `rustls_take_received_plaintext`。缺少任一 hook point 时必须停止，且不得改用非生产二进制绕过检查。

## 2. 检查配置并启动

默认生成配置已经声明 `tls-plaintext-payload`，并设置 `[payload.tls] enabled = true`、`capture_backend = "tls-sync"`、`source/resolver/library = "auto"`。若使用自定义配置，显式核对这些字段以及 `sync_runtime_library_path`、`sync_event_socket_path` 和 retention/redaction 边界。

```bash
sudo ./target/release/actraild start
sudo ./target/release/actrailctl doctor
```

## 3. 运行 xiaoO

```bash
./target/release/actrailctl launch \
  --name xiaoo-rustls-llm \
  -- \
  "$XIAOO_BINARY" --cli run \
    --no-tools \
    --max-turns 1 \
    --prompt "请用一句话回答：AcTrail 正在观测哪个 TLS provider？"
```

xiaoO 应保留现有用户配置和最小化 API-key environment。整个 shell environment 不得仅为连接 daemon 而复制给更高权限身份。

## 4. 查看证据

```bash
sudo ./target/release/actrailctl list-traces
sudo ./target/release/actrailviewer payloads --trace-id <TRACE_ID> --head 20
sudo ./target/release/actrailviewer actions --trace-id <TRACE_ID>
sudo ./target/release/actrailviewer diagnostics --trace-id <TRACE_ID>
```

成功的原始证据应来自 `TlsUserSpace`、library `rustls` 和相应 hook symbol；完整 HTTP 请求可进一步投影为 `llm.request`。缺失时按 [采集结果缺失](../troubleshooting/capture.md) 排查。
