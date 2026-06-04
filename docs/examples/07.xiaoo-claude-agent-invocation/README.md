# xiaoO 调用 Claude Code 的进程语义验证

这个示例验证真实 xiaoO agent 静默拉起 `claude -p ...` 时，AcTrail 能否在进程层识别 agent invocation 关系。

这是 process/semantic trace 示例，不是 LLM payload capture 示例。`payload_tls_enabled = false` 和 `payload_socket_enabled = false` 是有意设置。通过条件在导出的 OpenTelemetry trace 里：必须存在一条 seccomp 观测到的 `claude -p ...` `process.exec` span，以及一条 parent 为 xiaoO、child 为 Claude Code 的 `agent.invocation` span。

## 文件

| 文件 | 用途 |
| --- | --- |
| `operator.conf` | AcTrail operator 配置，用于 process exec seccomp observation 和 agent invocation 语义组装。 |
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

第一条命令至少应输出 `1`。第二条命令必须非零。`agent.invocation` span attributes 里应包含以 `xiaoo` 结尾的 `agent.parent.executable`，以及包含 `claude -p` 的 `agent.child.command_line`。

## 这个示例证明什么

- `actrailctl launch` 会让 AcTrail 成为 trace root。
- 被启动的 xiaoO 进程会继承 trace。
- Claude Code 的启动会被 launch-time `execve`/`execveat` seccomp notify 观测到。
- AcTrail 会把底层 process facts 组装成更高层的 `agent.invocation` semantic action。

这个示例不校验 Claude 的自然语言回答，也不采集 Claude LLM request bytes。xiaoO 和 Claude Code 可能通过 HTTPS/TLS 或 plain HTTP 访问 provider；完整 payload 采集请优先参考 `docs/examples/08.full-monitor-validation/`。只验证 xiaoO outbound request payload 时，可参考 `docs/examples/06.xiaoo-tls-capture/`。
