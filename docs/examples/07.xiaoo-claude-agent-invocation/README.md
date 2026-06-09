# xiaoO 调用 Claude Code 的进程语义验证

这个示例验证真实 xiaoO agent 静默拉起 `claude -p ...` 时，AcTrail 能否在识别到 Claude 的 LLM 调用后，把 Claude 进程标记为 agent，并把它的直接拉起边升级为 `agent.invocation`。

这是 process/semantic trace 加 TLS plaintext evidence 示例。`payload_tls_enabled = true` 是必要条件，因为 `agent.invocation` 不再由命令名直接触发；它需要先从 Claude 的出站 LLM request 生成 `llm.request` 证据。通过条件在导出的 OpenTelemetry trace 里：必须存在一条 seccomp 观测到的 `claude -p ...` `process.exec` span、一条同 pid 的 `llm.request` span，以及一条 child 为 Claude Code 的 `agent.invocation` span。`agent.invocation` 的 parent 是 Claude 的直接拉起进程，可能是 xiaoO，也可能是 xiaoO 工具链中的 bash/timeout/python wrapper。

`operator.conf` 继承了 xiaoO TLS 捕获示例里的 stdio、socket 和 application HTTP/SSE 投影能力，并额外打开 process exec seccomp observation 和 agent invocation projection。不要把这些 payload/application 项裁剪成只剩 `process_seccomp`：xiaoO 的第一轮 LLM 决策本身也要被完整观测，否则示例可能停在 xiaoO 第一轮，既看不到 Claude 子进程，也不会生成后续 `agent.invocation` 证据。这个示例不配置 `agent_invocation_command`；入口 xiaoO 可以由 launch 命令本身预热，Claude 子进程必须通过 runtime -> daemon 动态 lookup 得到 TLS sync probe plan。`payload_tls_ring_buffer_bytes = 8388608`、SSE/data preview 上限 `65536` 以及 socket/event ring buffer 值和 xiaoO TLS 示例保持一致，用来覆盖真实 streaming LLM 响应和工具调用 delta。

## 文件

| 文件 | 用途 |
| --- | --- |
| `operator.conf` | AcTrail operator 配置，用于 process exec seccomp observation、TLS plaintext payload capture 和 agent invocation 语义组装。 |
| `workload.conf` | 真实 xiaoO -> Claude Code 运行所需的 workload 参数。 |
| `agent_prompt.template` | 提示词模板，要求 xiaoO 只执行一次前台 `claude -p ...` 命令。 |
| `run_e2e.py` | 最薄的 docs 入口，调用共享真实 E2E runner 并传入本示例配置。 |

## 前置条件

- 在仓库根目录用 root shell 运行。
- 先完成 release 构建：`cargo build --release`。
- `xiaoo` 在 `PATH` 中；`which xiaoo` 应打印将要启动的可执行文件。只有测试特定非 PATH binary 时，才修改 `workload.conf` 里的 `agent_command`。
- `claude` 在 `PATH` 中，并且当前 shell 的认证状态可以让 `claude -p ...` 正常回答。
- Claude Code 可以访问外部网络。

不要手工编辑 `/tmp` 路径。使用 cleanup helper，或依赖 E2E 脚本内置的 `actrailctl clean` 步骤。

`workload.conf` 里的 `claude_timeout_seconds` 是 xiaoO 要求 bash 执行的内层 `timeout`；`launch_timeout_seconds` 是外层 `actrailctl launch` 等待 xiaoO 返回的时间，必须大于内层 Claude timeout，否则外层可能在 Claude 刚结束前杀掉整个 workload。

## 运行

```bash
python3 docs/examples/clean.py --example xiaoo-claude
python3 docs/examples/07.xiaoo-claude-agent-invocation/run_e2e.py
```

期望终端输出包含：

```text
ACTRAIL_AGENT_TREE_OK
agent_invocation_trace_id=1
otel_output=/tmp/actrail-xiaoo-claude-agent-invocation.otlp.json
agent invocation e2e complete
```

如果缺少 xiaoO、Claude Code、AcTrail binaries、seccomp/eBPF 权限，或导出的 OTEL span 不符合预期，脚本会 fail-fast。

## 手动检查

成功运行后，可以检查导出的 OTLP JSON：

```bash
jq '[.resourceSpans[].scopeSpans[].spans[] | select(any(.attributes[]?; .key=="actrail.action.kind" and .value.stringValue=="agent.invocation"))] | length' \
  /tmp/actrail-xiaoo-claude-agent-invocation.otlp.json

jq '[.resourceSpans[].scopeSpans[].spans[] | select(any(.attributes[]?; .key=="seccomp_observed" and .value.stringValue=="true"))] | length' \
  /tmp/actrail-xiaoo-claude-agent-invocation.otlp.json
```

第一条命令至少应输出 `1`。第二条命令必须非零。`agent.invocation` span attributes 里应包含 `agent.child.command_line` 且其中有 `claude -p`；`agent.parent.*` 表示 Claude 的直接拉起进程，不要求一定是 xiaoO。

## 这个示例证明什么

- `actrailctl launch` 会让 AcTrail 成为 trace root。
- 被启动的 xiaoO 进程会继承 trace。
- Claude Code 的启动会被 launch-time `execve`/`execveat` seccomp notify 观测到。
- Claude Code 的出站 LLM request 会形成 `llm.request` 证据。
- AcTrail 会把发生 LLM 调用的 Claude 进程标记为 agent，并把它的直接 parent -> child exec 边升级成 `agent.invocation` semantic action。

这个示例不校验 Claude 的自然语言回答质量。xiaoO 和 Claude Code 可能通过 HTTPS/TLS 或 plain HTTP 访问 provider；完整 payload 采集请优先参考 `docs/examples/08.full-monitor-validation/`。只验证 xiaoO outbound request payload 时，可参考 `docs/examples/06.xiaoo-tls-capture/`。
