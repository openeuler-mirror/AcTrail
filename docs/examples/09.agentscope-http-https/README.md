# AgentScope 类 Agent 观测用例

本用例面向负责部署和验收 AcTrail 的企业运维人员，说明 AcTrail 如何观测 AgentScope 这类 Python agent 框架的运行过程。AgentScope agent 的业务代码只依赖框架的 `Agent`、`OpenAIChatModel`、`OpenAICredential` 和 `UserMsg`；AcTrail 从 agent 拉起、进程树、标准输入输出、网络明文、HTTP/SSE 协议事件和 LLM 语义动作等层面生成审计证据。

本用例使用 AcTrail 默认全量采集配置 `/etc/actrail/actraild.conf`。该配置由 `actraild init` 生成，覆盖进程、stdio、socket payload、TLS plaintext、HTTP/SSE 协议事件和 LLM 语义动作等观测能力。

## 观测目标

一条 AcTrail trace 表示一次由 `actrailctl launch` 启动并跟踪的进程树。对 AgentScope 类 agent，验收对象不是单个 HTTP 请求，而是一次 agent 运行的完整行为链：

| 观测面 | 用途 | 关键证据 |
| --- | --- | --- |
| Agent 拉起 | 确认 AcTrail 成为 agent 进程树的观测入口。 | trace root、进程启动命令、进程生命周期、退出状态。 |
| Agent 运行输出 | 确认 agent 执行结果和工具行为可用于验收。 | `actrailctl launch` 输出中的 AgentScope event stream、AcTrail `command.invocation`、工具子进程、trace health。 |
| LLM provider 访问 | 确认 agent 调用模型服务的请求与响应可还原为协议和语义证据。 | 网络 plaintext payload、HTTP/SSE 事件、`llm.request`、`llm.response`。 |
| 工具调用 | 当 agent workload 调用本地命令、脚本或外部工具时，确认工具行为归入同一 trace。 | 子进程生命周期、argv、stdio、文件访问、网络连接，以及工具产生的后续 payload。 |

本目录的基线 workload 使用 AgentScope `Toolkit` 注册 `Bash`、`Glob`、`Grep`、`Read`、`Write`、`Edit`，并通过 `reply_stream` 输出 AgentScope 事件。工具工作目录为 `target/docs-examples/agentscope-http-https/tool-workspace`；非交互执行时，`agent.py` 使用 AgentScope 的 `ACCEPT_EDITS` 权限模式，允许本用例的工具命令在该目录内完成。HTTP route 使用仓库已有的 OpenAI-compatible reverse provider proxy，把 AgentScope 的 plain HTTP 请求转发到真实 HTTPS upstream；HTTPS route 由 AgentScope 直连同一个 OpenAI-compatible upstream。两条 route 的示例 prompt 都会触发一次 `Bash` 工具调用，同一 trace 中应看到工具调用证据。

同一默认部署配置需要完成两类 provider route 的观测：

| Route | 用途 | 关键证据 |
| --- | --- | --- |
| HTTP provider route | 验证 AgentScope 访问 plain HTTP reverse provider proxy 时的 socket plaintext 采集。 | `Syscall socket-syscall` payload、HTTP request/response 事件、`llm.response` 和 SSE semantic action。 |
| HTTPS provider route | 验证 HTTPS 场景下的 TLS plaintext 采集。 | `TlsUserSpace` payload、HTTP/SSE 事件、`llm.request` 和 `llm.response` semantic action。 |

本文中的 payload 指 AcTrail 持久化的明文字节片段，可以来自网络、TLS 边界或 stdio。`Syscall socket-syscall` 是普通 socket 读写路径上的明文证据，`TlsUserSpace` 是 TLS 库明文边界上的证据。semantic action 是 AcTrail 从底层 payload 和协议事件归纳出的高层行为，例如 `llm.request`、`llm.response`。

## 文件

必须文件：

| 文件 | 用途 |
| --- | --- |
| `agent.py` | AgentScope 原生 workload。它读取 OpenAI-compatible base URL、模型名和 API key 环境变量，注册 AgentScope Toolkit，并用 `reply_stream` 输出模型调用、工具调用和最终文本事件。 |

HTTP route 支撑文件：

| 文件 | 用途 | 替代关系 |
| --- | --- | --- |
| `tests/support/llm-http-proxy/provider_proxy.py` | 仓库已有的 OpenAI-compatible reverse provider proxy。它给 AgentScope 暴露 plain HTTP base URL，并把请求转发到真实 HTTPS upstream。 | 替代企业现场已有的 plain HTTP OpenAI-compatible provider endpoint。现场已经有 plain HTTP endpoint 时，HTTP route 的 `--api-url` 可直接指向该 endpoint。 |

可选文件（现场需要时生成）：

| 文件 | 用途 | 替代关系 |
| --- | --- | --- |
| `local/agentscope-actraild.conf` | 生成到本地路径的 AcTrail operator config。 | 替代默认 `/etc/actrail/actraild.conf`；后续 `actraild`、`actrailctl`、`actrailviewer` 命令中的 config 路径同步替换。 |
| `local/agentscope.patch.toml` | 站点级配置片段，用于在初始化配置时写入本机路径、保留策略或其他部署差异。 | 替代手工编辑生成后的 operator config。 |

## 前置条件

在仓库根目录构建 release 二进制：

```bash
cargo build --release
```

确认当前 uv 环境能导入 AgentScope 及其 HTTP 依赖：

```bash
export ACTRAIL_UV_BIN="$(command -v uv)"
"$ACTRAIL_UV_BIN" run python -c 'import agentscope, openai, httpx; print(agentscope.__version__)'
```

`actrailctl launch` 直接执行命令 argv；在 `sudo -E` 下，sudo 可能把 `PATH` 改成系统默认路径。后续 launch 命令使用 `ACTRAIL_UV_BIN` 的绝对路径，保证 child exec 使用同一个 uv 环境。

生成或校验默认配置：

```bash
sudo target/release/actraild init
```

启动或复用默认 daemon：

```bash
sudo target/release/actrailctl --config /etc/actrail/actraild.conf doctor || \
  sudo target/release/actraild --config /etc/actrail/actraild.conf start
sudo target/release/actrailctl --config /etc/actrail/actraild.conf doctor
```

需要把配置写到其他路径时，使用 `--output` 指定目标文件：

```bash
sudo target/release/actraild init --output local/agentscope-actraild.conf
```

已有站点级配置片段时，可在初始化时传入 `--patch`：

```bash
sudo target/release/actraild init \
  --output local/agentscope-actraild.conf \
  --patch local/agentscope.patch.toml
```

后续命令中的 `/etc/actrail/actraild.conf` 替换为实际配置路径即可。

## Provider 参数

下面的示例使用 DeepSeek OpenAI-compatible endpoint。替换为企业内 provider 时，保持 `ACTRAIL_AGENTSCOPE_PROVIDER_BASE_URL` 为 OpenAI SDK 使用的 base URL。

```bash
export ACTRAIL_AGENTSCOPE_PROVIDER_BASE_URL='https://api.deepseek.com'
export ACTRAIL_AGENTSCOPE_PROVIDER_MODEL='deepseek-v4-flash'
export DEEPSEEK_API_KEY='<api-key>'
```

## HTTP Route

在终端 A 启动仓库已有的 plain HTTP reverse provider proxy：

```bash
"$ACTRAIL_UV_BIN" run python tests/support/llm-http-proxy/provider_proxy.py \
  --mode forward \
  --bind-host 127.0.0.1 \
  --bind-port 18098 \
  --upstream-base-url "$ACTRAIL_AGENTSCOPE_PROVIDER_BASE_URL" \
  --upstream-api-key-env DEEPSEEK_API_KEY \
  --upstream-auth-header-name Authorization \
  --upstream-auth-scheme Bearer
```

在终端 B 通过 AcTrail 启动 AgentScope agent：

```bash
export ACTRAIL_AGENTSCOPE_HTTP_API_KEY=actrail-local-proxy-key

sudo -E target/release/actrailctl --config /etc/actrail/actraild.conf launch \
  --name agentscope-http \
  -- "$ACTRAIL_UV_BIN" run python docs/examples/09.agentscope-http-https/agent.py \
    --prompt 'Use the Bash tool once to run: printf actrail-agent-tool-ok. Then answer exactly ACTRAIL_AGENTSCOPE_HTTP_OK.' \
    --model "$ACTRAIL_AGENTSCOPE_PROVIDER_MODEL" \
    --api-url http://127.0.0.1:18098 \
    --api-key-env ACTRAIL_AGENTSCOPE_HTTP_API_KEY
```

命令输出中会出现 `trace trace-<N> entered Active`。记录实际 trace id，后续命令用 `<HTTP_TRACE_ID>` 表示。

HTTP route 中，AgentScope 只连接 `http://127.0.0.1:18098`，AcTrail 对 AgentScope 进程看到的是 plain HTTP socket payload。proxy 进程不在这条 trace 的进程树内，它负责把请求转发到 `ACTRAIL_AGENTSCOPE_PROVIDER_BASE_URL`。

## HTTPS Route

通过 AcTrail 启动 AgentScope agent，直接访问同一个 HTTPS provider：

```bash
export ACTRAIL_AGENTSCOPE_HTTPS_API_KEY="$DEEPSEEK_API_KEY"

sudo -E target/release/actrailctl --config /etc/actrail/actraild.conf launch \
  --name agentscope-https \
  -- "$ACTRAIL_UV_BIN" run python docs/examples/09.agentscope-http-https/agent.py \
    --prompt 'Use the Bash tool once to run: printf actrail-agent-tool-ok. Then answer exactly ACTRAIL_AGENTSCOPE_HTTPS_OK.' \
    --model "$ACTRAIL_AGENTSCOPE_PROVIDER_MODEL" \
    --api-url "$ACTRAIL_AGENTSCOPE_PROVIDER_BASE_URL" \
    --api-key-env ACTRAIL_AGENTSCOPE_HTTPS_API_KEY
```

记录输出中的 trace id，后续命令用 `<HTTPS_TRACE_ID>` 表示。

## 查看证据

列出 trace：

```bash
sudo target/release/actrailctl --config /etc/actrail/actraild.conf list-traces
```

查看 trace 摘要：

```bash
sudo target/release/actrailviewer --config /etc/actrail/actraild.conf summary --trace-id <TRACE_ID>
```

查看进程与命令证据：

```bash
sudo target/release/actrailviewer --config /etc/actrail/actraild.conf processes --trace-id <TRACE_ID>
```

查看 payload：

```bash
sudo target/release/actrailviewer --config /etc/actrail/actraild.conf payloads --trace-id <TRACE_ID> --head 160
```

查看应用层事件：

```bash
sudo target/release/actrailviewer --config /etc/actrail/actraild.conf events --trace-id <TRACE_ID> --head 120
```

查看语义动作：

```bash
sudo target/release/actrailviewer --config /etc/actrail/actraild.conf actions --trace-id <TRACE_ID> --head 120
```

查看诊断：

```bash
sudo target/release/actrailviewer --config /etc/actrail/actraild.conf diagnostics --trace-id <TRACE_ID>
```

导出 OTEL JSON：

```bash
sudo target/release/actrailviewer --config /etc/actrail/actraild.conf export-otel \
  --trace-id <TRACE_ID> \
  --output /tmp/agentscope-trace.otlp.json
```

## 验收标准

HTTP trace 使用 `<HTTP_TRACE_ID>` 检查：

- `summary` 显示 trace lifecycle 为 completed，health 为 clean。
- `processes` 显示 `actrailctl launch` 拉起的 uv/Python/AgentScope workload 进程。
- `actrailctl launch` 命令输出包含 `agentscope_event=model_call_start`、`agentscope_event=tool_call_start`、`agentscope_event=tool_result_text_delta`、`actrail-agent-tool-ok` 和 `agentscope_final_text=ACTRAIL_AGENTSCOPE_HTTP_OK`。
- `payloads` 中存在 complete/success 的 outbound 与 inbound `Syscall socket-syscall` payload。
- `events` 中存在 HTTP request/response 事件。
- `actions` 中存在 success/complete 的 `http.message`、`sse.stream` 和 `llm.response`。
- `processes` 和 `actions` 中存在工具子进程及其 `command.invocation`，命令为 `/bin/sh -c printf actrail-agent-tool-ok`。

HTTPS trace 使用 `<HTTPS_TRACE_ID>` 检查：

- `summary` 显示 trace lifecycle 为 completed，health 为 clean。
- `processes` 显示 `actrailctl launch` 拉起的 uv/Python/AgentScope workload 进程。
- `actrailctl launch` 命令输出包含 `agentscope_event=tool_call_start`、`agentscope_event=tool_result_text_delta`、`actrail-agent-tool-ok` 和 `agentscope_final_text=ACTRAIL_AGENTSCOPE_HTTPS_OK`。
- `payloads` 中存在 complete/success 的 outbound 与 inbound `TlsUserSpace` payload。
- `events` 中存在 HTTPS 解密后的 HTTP/SSE 事件。
- `actions` 中存在 success/complete 的 `llm.request` 与 `llm.response`。
- `processes` 和 `actions` 中存在工具子进程及其 `command.invocation`，命令为 `/bin/sh -c printf actrail-agent-tool-ok`。
- `diagnostics` 没有 TLS probe、TLS runtime 或 payload analyzer 错误。

HTTP 与 HTTPS 两条 trace 同时满足上述标准，才表示本用例在当前主机和默认配置上通过。

## 运维排查

| 现象 | 处理 |
| --- | --- |
| `uv run python` 无法导入 `agentscope`、`openai` 或 `httpx` | 在当前 uv 项目环境安装缺失包，然后重新执行前置条件检查。 |
| `actrailctl launch` 阶段报告 TLS probe 或 TLS runtime 错误 | 先记录完整命令、目标 Python 路径和错误输出，再用 `target/release/tls-probe-point-finder fast --provider auto --source auto "$(readlink -f .venv/bin/python3)"` 核验实际运行路径的 TLS probe plan。 |
| HTTP trace 缺少 `Syscall socket-syscall` payload | 检查默认配置中的 socket payload 采集项，以及 reverse provider proxy 是否仍在监听 `127.0.0.1:18098`。 |
| HTTPS trace 缺少 `TlsUserSpace` payload | 检查默认配置中的 `payload.tls.enabled`、`payload.tls.capture_backend`、`payload.tls.source`、`payload.tls.resolver` 和 `payload.tls.library`，同时查看 `actrailviewer diagnostics` 与 `/var/log/actrail/actraild.log`。 |
| HTTPS trace 缺少 `llm.request` 或 `llm.response`，或 HTTP trace 缺少 `llm.response` | 先确认对应 payload 完整，再检查默认配置中的 HTTP、HTTP/2、SSE analyzer 是否启用。 |

## 清理

停止 HTTP reverse provider proxy：

```bash
# 在终端 A 按 Ctrl-C
```

停止默认 daemon：

```bash
sudo target/release/actraild --config /etc/actrail/actraild.conf stop
```

默认 SQLite 证据保留在 `/var/lib/actrail/actrail.sqlite`，用于后续审计和导出。
