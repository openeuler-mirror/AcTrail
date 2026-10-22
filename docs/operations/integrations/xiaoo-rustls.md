# 捕获 xiaoO rustls LLM 请求

> 本文说明如何在不修改 xiaoO 源码的情况下捕获 rustls HTTPS 明文并查看 `llm.request`。

本流程使用默认的 `bpf-copy` 后端，通过 `actrailctl launch` 在程序启动前发现并挂载 TLS 探针。

## 前置条件

- **daemon 状态**：AcTrail 默认配置已生成，daemon 尚未启动或可安全重启。
- **探针工具**：`target/release/tls-probe-point-finder` 已构建。
- **目标程序**：生产实际使用的 `xiaoo` 位于 `PATH`。
- **模型与认证**：xiaoO 自身的 provider/model/API-key 环境已经配置，且 API key 未写入 argv。
- **访问权限**：运行 `actrailctl` 的用户有权访问 operator config 指定的 control socket。

命令从 AcTrail 仓库根目录运行，因为 probe point finder 与 control binary 使用 `target/release/` 中本次构建的产物。

## 1. 验证目标二进制的自动探针计划

```bash
export XIAOO_BINARY=/absolute/path/to/xiaoo
test -x "$XIAOO_BINARY"

./target/release/tls-probe-point-finder fast \
  --provider rustls \
  --source auto \
  --match-limit 8 \
  "$XIAOO_BINARY"
```

输出应包含 `provider = rustls`、`rustls_buffer_plaintext` 和 `rustls_take_received_plaintext`，分别确认 TLS 实现、发送和接收明文探针。缺少任一探针时，先按 [采集结果缺失](../troubleshooting/capture.md) 排查该二进制的探针识别，再继续启动。

## 2. 检查配置并启动

- **TLS 后端**：默认生成配置已经声明 `tls-plaintext-payload`，并设置 `[payload.tls] enabled = true`、`capture_backend = "bpf-copy"`、`source/resolver/library = "auto"`。
- **seccomp 设置**：seccomp-notify 默认关闭。
- **探针发现**：使用自定义配置时，核对这些字段，并确认 `[payload.tls]` 下的 `direct_startup_discovery_enabled` 和 `direct_dynamic_discovery_enabled` 均为 `true`，分别启用启动时和运行中的探针发现。
- **容量与保留**：采集容量、内容保留与脱敏字段见 [TLS 采集配置](../../reference/configuration/collection.md#payloadtls)。

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

以有权访问 daemon control socket 的用户运行 xiaoO，使用该用户的模型配置及所需的 API-key 环境变量。

## 4. 查看证据

```bash
sudo ./target/release/actrailctl list-traces
sudo ./target/release/actrailviewer payloads --trace-id <TRACE_ID> --head 20
sudo ./target/release/actrailviewer actions --trace-id <TRACE_ID>
sudo ./target/release/actrailviewer diagnostics --trace-id <TRACE_ID>
```

- **原始证据**：成功的原始证据应来自 `TlsUserSpace`、library `rustls` 和相应 hook symbol。
- **语义结果**：完整 HTTP 请求可进一步投影为 `llm.request`。
- **故障排查**：缺失时按 [采集结果缺失](../troubleshooting/capture.md) 排查。
