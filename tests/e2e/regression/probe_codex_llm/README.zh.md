# 回归测试：Codex LLM 交互采集

## 测试目标

验证 AcTrail 能通过 eBPF 采集 Codex 的模型请求和响应，并将它们关联到对应的 LLM 调用。

## 场景设计

在隔离的 AcTrail 环境中启动一次 Codex 任务，刷新默认采集配置后，在守护进程启动前登记已知的 Codex 原生可执行文件。通过正常 Codex CLI 提出一个简短的开放式问题，以 Codex 实际返回的助手文本核对响应正文。按启动时取得的 trace ID 查询采集结果。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、可工作的 eBPF 环境、root 权限 | 采集 Codex 活动 |
| AcTrail | 可运行的 release 产物 `actraild`、`actrailctl`、`actrailviewer` | 启动采集、保存并查询观测记录 |
| Codex | 支持 JSON 事件输出和 `exec --output-last-message`，且可定位原生可执行文件 | 执行文本任务，提供实际回复和采集目标 |
| 模型服务 | 已配置认证，服务可访问 | 执行真实的模型请求 |

## 测试步骤摘要

1. **检查 Codex 可用性。** 直接运行 Codex 完成一次模型请求。Codex 缺失或可用性检查失败时，用例标记为 `SKIPPED`。
2. **准备观测环境。** 在测例工作目录用 `actrailctl init` 生成默认配置，并登记 Codex 原生可执行文件。用 `actraild start` 启动守护进程。
3. **启动 Codex。** 通过 `actrailctl launch` 启动 `codex exec`，请求自动选择 eBPF 和 seccomp-notify。执行器根据 CLI 帮助检测并按需传入 `--no-daemon`。记录启动结果中的 trace ID 和权限选择。
4. **执行文本任务。** 提交开放式问题。等待任务成功结束，读取 Codex 输出的本轮最终回复文本。
5. **完成数据采集。** Codex 退出后停止守护进程，等待观测记录落盘。
6. **核对启动权限和观测记录。** 核对第 3 步的权限选择结果。使用 `actrailviewer` 查询对应的 trace ID。

   | 检查项 | 条件 | 预期结果 |
   | --- | --- | --- |
   | 启动权限 | AcTrail 自动选择采集权限 | eBPF 已启用，seccomp-notify 已关闭，启动未降级 |
   | Trace 终态 | 守护进程已停止 | trace 状态为 `Completed` 或 `Exited`，健康状态为 `Clean` |
   | 调用关联 | trace 下的 LLM 请求和响应 | 每条 `llm.call` 关联一条 `llm.request`，最多关联一条 `llm.response`。每条请求或响应仅关联到一条调用。无响应的调用保持 `unknown/partial`，无结束时间 |
   | 任务内容 | Codex 成功完成任务并返回文本 | 至少一条 `success/complete` 响应的正文包含实际回复全文。同一调用的请求保留规范化内容，或在采集受限时保留有效的请求元数据 |
   | 采集收尾 | 守护进程已停止 | LLM 请求、响应和 SSE 流处于终态 |

## 快速运行

### 测试所用环境变量

| 环境变量 | 未设置时测试框架的行为 | 用途 |
| --- | --- | --- |
| `CODEX_E2E_COMMAND_TIMEOUT_SECONDS` | 等待 30 秒 | AcTrail 命令的等待时限 |
| `CODEX_E2E_LAUNCH_TIMEOUT_SECONDS` | 等待 180 秒 | Codex 文本任务的等待时限 |
| `CODEX_E2E_BINARY` | 自动发现 Codex 可执行文件 | 指定 Codex CLI 路径 |
| `CODEX_E2E_NATIVE_BINARY` | 从选定的 Codex CLI 定位原生可执行文件 | 指定启动前登记的采集目标 |
| `CODEX_E2E_MODEL` | 使用 `codex debug models` 返回的首个未隐藏且支持 API 的模型。查询无结果时使用 `gpt-5.5` | 指定本次测试使用的模型 |
| `CODEX_E2E_REASONING_EFFORT` | 使用 `low` | 指定本次测试的推理强度 |
| `ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS` | 等待 60 秒 | Codex 可用性检查的等待时限 |

### 运行指令

从仓库根目录执行以下命令。

```bash
sudo -E python3 tests/e2e/regression/probe_codex_llm/run_e2e.py
```
