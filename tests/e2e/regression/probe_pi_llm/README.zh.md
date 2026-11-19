# 回归测试：Pi LLM 交互采集

## 测试目标

验证 AcTrail 能采集 Pi 的模型请求和响应，并将它们关联到对应的 LLM 调用。

## 场景设计

在隔离的 AcTrail 环境中启动 Pi 的 RPC 会话，使用刷新后的默认采集配置。Pi 沿用自身的模型和服务配置，执行一次简短的开放式文本任务。以 Pi 实际返回的助手文本核对采集响应正文。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、可工作的 eBPF 环境、root 权限 | 采集 Pi 活动 |
| AcTrail | 可运行的 release 产物 `actraild`、`actrailctl`、`actrailviewer` | 启动采集、保存并查询观测记录 |
| Pi | 可从 `PATH` 找到，且支持 RPC 的 CLI | 执行文本任务并返回助手事件 |
| 模型服务 | Pi 已配置默认模型和认证，服务可访问 | 执行真实的模型请求 |

## 测试步骤摘要

1. **检查 Pi 可用性。** 直接运行 Pi 完成一次模型请求。Pi 缺失或可用性检查失败时，用例标记为 `SKIPPED`。
2. **准备观测环境。** 在测例工作目录用 `actrailctl init` 生成默认配置。用 `actraild start` 启动守护进程。
3. **启动 Pi 会话。** 通过 `actrailctl launch` 启动 Pi RPC 会话，关闭默认工具、内置 MCP 和会话保存。记录启动结果中的 trace ID。
4. **执行文本任务。** 通过 RPC 提交一个简短的开放式问题。等待任务成功结束，从 `agent_end` 事件取得本轮最终助手回复文本。
5. **完成数据采集。** 结束 Pi 会话后停止守护进程，等待观测记录落盘。
6. **核对观测记录。** 使用 `actrailviewer` 查询第 3 步取得的 trace ID。

   | 检查项 | 条件 | 预期结果 |
   | --- | --- | --- |
   | Trace 终态 | 守护进程已停止 | trace 状态为 `Completed` 或 `Exited`，健康状态为 `Clean` |
   | 调用关联 | trace 下的 LLM 请求和响应 | 每条 `llm.call` 关联一条 `llm.request`，最多关联一条 `llm.response`。每条请求或响应仅关联到一条调用。无响应的调用保持 `unknown/partial`，无结束时间 |
   | 任务内容 | Pi 成功完成任务并返回文本 | 至少一条 `success/complete` 响应的正文包含实际回复全文。同一调用的请求保留规范化内容，或在采集受限时保留有效的请求元数据 |
   | 采集收尾 | 守护进程已停止 | LLM 请求、响应和 SSE 流处于终态 |

## 快速运行

### 测试所用环境变量

| 环境变量 | 未设置时测试框架的行为 | 用途 |
| --- | --- | --- |
| `PI_E2E_COMMAND_TIMEOUT_SECONDS` | 等待 30 秒 | AcTrail 命令的等待时限 |
| `PI_E2E_LAUNCH_TIMEOUT_SECONDS` | 等待 180 秒 | Pi 文本任务的等待时限 |
| `ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS` | 等待 60 秒 | Pi 可用性检查的等待时限 |

### 运行指令

从仓库根目录执行以下命令。

```bash
sudo -E env "HOME=$HOME" "PATH=$PATH" \
  python3 tests/e2e/regression/probe_pi_llm/run_e2e.py
```
