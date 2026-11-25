# 回归测试：Claude LLM 交互采集

## 测试目标

验证 AcTrail 能采集 Claude 发出的模型请求和收到的响应，并将它们关联到对应的 LLM 调用。

## 场景设计

通过 AcTrail 启动 Claude 流式会话，执行一次真实的文本任务。以 Claude 实际返回的文本作为响应正文的核对依据。AcTrail 停止后，按启动时取得的 trace ID 核对落盘的请求、响应及其关联。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、可工作的 eBPF 环境、root 权限 | 采集 Claude 活动 |
| AcTrail | 可运行的 release 产物 `actraild`、`actrailctl`、`actrailviewer` | 启动采集、保存并查询观测记录 |
| Claude | 支持流式 JSON 输入和输出的 CLI | 执行文本任务并返回实际回复 |
| 模型服务 | 已配置认证，服务可访问 | 执行真实的模型请求 |

## 测试步骤摘要

1. **检查 Claude 可用性。** 直接运行 Claude 完成一次模型请求。Claude 缺失或可用性检查失败时，用例标记为 `SKIPPED`。
2. **准备观测环境。** 用 `actrailctl init` 刷新默认配置。用 `actraild start` 启动守护进程。
3. **启动 Claude 会话。** 通过 `actrailctl launch` 启动 Claude，以 CLI 参数关闭工具。记录启动结果中的 trace ID。
4. **执行文本任务。** 向 Claude 提出一个简短的开放式问题。等待任务成功结束，取得 Claude 的实际回复文本。
5. **完成数据采集。** 结束 Claude 会话后停止守护进程，等待观测记录落盘。
6. **核对观测记录。** 使用 `actrailviewer` 查询第 3 步取得的 trace ID。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| Trace 终态 | 守护进程已停止 | trace 状态为 `Completed` 或 `Exited`，健康状态为 `Clean` |
| 调用关联 | trace 下的 LLM 请求和响应 | 至少有一组请求和响应。每条 `llm.call` 关联一条 `llm.request`，并最多关联一条 `llm.response`。所有请求和响应都有对应的调用记录 |
| 任务内容 | Claude 成功完成任务并返回文本 | 至少一条响应为 `success/complete`，正文包含第 4 步取得的完整回复文本。同一调用的请求保留规范化内容，或在采集受限时保留有效的请求元数据 |
| 采集收尾 | 守护进程已停止 | LLM 请求、响应与 SSE 流均已收尾。因对端提前关闭（`PeerClosed`）而保留 `error/partial` 的响应可用同一 trace、进程和流中已结束的入站 HTTP 记录核实 |

## 快速运行

### 测试所用环境变量

| 环境变量 | 未设置时测试框架的行为 | 用途 |
| --- | --- | --- |
| `CLAUDE_E2E_COMMAND_TIMEOUT_SECONDS` | 等待 30 秒 | AcTrail 命令的等待时限 |
| `CLAUDE_E2E_LAUNCH_TIMEOUT_SECONDS` | 等待 180 秒 | Claude 文本任务的等待时限 |
| `CLAUDE_E2E_BINARY` | 自动发现 Claude 可执行文件 | 指定 Claude 可执行文件路径 |
| `CLAUDE_E2E_MODEL` | 使用 `ANTHROPIC_MODEL`，未设置时使用 `sonnet` | 指定本次测试使用的模型 |
| `ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS` | 等待 60 秒 | Claude 可用性检查的等待时限 |

### 运行指令

从仓库根目录执行以下命令。

```bash
sudo -E python3 tests/e2e/regression/probe_claude_llm/run_e2e.py
```
