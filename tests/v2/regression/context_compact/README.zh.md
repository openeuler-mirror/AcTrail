# context compaction 真实 Agent 回归

## 运行

在仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/context_compact/run_e2e.py
```

或使用统一入口：

```bash
sudo -E python3 tests/v2/regression/test_all.py --case context_compact
```

默认按 `opencode,claude` 顺序选择第一个已安装且可用的真实 Agent。
通过 `CONTEXT_COMPACT_E2E_AGENTS=claude` 或 `opencode` 可指定 Agent。
通过 `CONTEXT_COMPACT_E2E_MODE=disabled` 验证总开关关闭后的会话行为；默认 `inference` 验证压缩推断。
运行入口负责准备 release 可执行文件、发现 Agent 并检查外部服务可用性。

## 流程与验收

1. 使用 `init -f` 刷新默认配置，按模式设置压缩总开关，启动隔离的 AcTrail runtime。
2. 在同一个 Agent 进程中完成首轮任务、原生 `/compact` 和后续一轮对话。
   OpenCode 使用原生 summarize API；Claude 使用交互会话。
3. 正常关闭 Agent，停止 daemon 并等待数据排空。
4. 按本次 Trace 查询持久化动作，检查 LLM 请求、响应及请求轨迹。
   `inference` 模式要求 `agent.context_compaction` 数量大于零，`disabled` 模式要求数量为零。

通过信息显示 `trace-N agent.context_compaction=<数量>`。
Agent 缺失或外部不可用时报告 `SKIPPED`；启动、会话、排空或动作验收失败时报告 `FAILED`。

TODO：接入能真实产生 `compaction_trigger` 与 `request_kind=compaction` 的 Agent，
补充 `inference_enabled=false` 的显式协议回归。

## 配置

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `CONTEXT_COMPACT_E2E_AGENTS` | `opencode,claude` | Agent 选择顺序 |
| `CONTEXT_COMPACT_E2E_MODE` | `inference` | `inference` 验证推断，`disabled` 验证总开关关闭 |
| `CLAUDE_E2E_BINARY` | 公共发现链 | Claude 可执行文件 |
| `CLAUDE_E2E_MODEL` | 公共 Claude 模型选择 | Claude 模型 |
| `OPENCODE_E2E_BINARY` | 公共发现链 | OpenCode 可执行文件 |
| `OPENCODE_E2E_MODEL` | OpenCode 默认模型 | 可指定 `provider/model` |
| `CONTEXT_COMPACT_E2E_STARTUP_TIMEOUT_SECONDS` | `60` | Agent 启动超时 |
| `CONTEXT_COMPACT_E2E_TURN_TIMEOUT_SECONDS` | `240` | 每轮对话超时 |
| `CONTEXT_COMPACT_E2E_SHUTDOWN_TIMEOUT_SECONDS` | `15` | Agent 退出超时 |
| `CONTEXT_COMPACT_E2E_TERMINAL_ROWS` | `40` | Claude 终端行数 |
| `CONTEXT_COMPACT_E2E_TERMINAL_COLUMNS` | `160` | Claude 终端列数 |

公共工作目录、命令超时及轮询配置见 [`../README.md`](../README.md)。
