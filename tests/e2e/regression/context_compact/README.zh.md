# 回归测试：上下文压缩识别
## 测试目标

验证 AcTrail 能识别 Agent 实际发生的上下文压缩。

## 场景设计

在一次连续的 Agent 会话中触发一次上下文压缩，验证 AcTrail 能从压缩前后的模型请求中识别这次压缩。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、可工作的 eBPF 环境、root 权限 | 采集 Agent 活动 |
| AcTrail | 可运行的 release 产物：`actraild`、`actrailctl`、`actrailviewer` | 采集、存储、查询观测记录 |
| Claude | 支持流式会话、手动上下文压缩的 CLI | 执行任务，提供压缩完成结果 |
| 模型服务 | 已配置认证，服务可访问 | 执行真实的模型请求 |

## 测试步骤摘要

1. **准备观测环境。** 先生成此次测试所用配置(`actrailctl init`)，使用刷新后的 AcTrail 默认配置启动守护进程(`actraild start`)。
2. **启动 Agent 会话。** 通过 AcTrail (`actrailctl launch`) 启动 Claude 流式会话。记录保存启动结果中的 trace ID。
3. **执行任务 1。** 在同一会话中完成多轮相关交互，为压缩提供已有对话内容。
4. **触发上下文压缩。** 调用 Agent 的压缩接口（通过 Agent 接口确保压缩成功）。
5. **执行任务 2。** 在同一会话中发起后续任务，使压缩后的上下文进入模型请求，为识别上下文变化提供观测数据。
6. **完成数据采集。** 结束 Agent 会话后停止 AcTrail，等待缓冲数据落盘。
7. **核对压缩识别结果。** 按第 2 步取得的 trace ID 查阅 AcTrail 观测记录。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 压缩记录数量 | Agent 完成一次上下文压缩，任务 2 发出压缩后的模型请求 | 第 2 步取得的 trace ID 下有且仅有一条 `agent.context_compaction` 记录 |

# 快速运行

## 测试所用环境变量

| 环境变量 | 未设置时测试框架的行为 | 用途 |
| --- | --- | --- |
| `CONTEXT_COMPACT_E2E_COMMAND_TIMEOUT_SECONDS` | 等待 30 秒 | AcTrail 命令的等待时限 |
| `CONTEXT_COMPACT_E2E_LAUNCH_TIMEOUT_SECONDS` | 等待 180 秒 | 每轮 Claude 任务的等待时限 |
| `CLAUDE_E2E_BINARY` | 自动发现 Claude 可执行文件 | 指定 Claude 可执行文件路径 |
| `CLAUDE_E2E_MODEL` | 沿用 Claude 的模型配置 | 覆盖本次测试使用的模型 |
| `ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS` | 等待 60 秒 | Claude 可用性探测的等待时限 |

## 运行指令

在仓库根目录执行：

```bash
sudo -E python3 tests/e2e/regression/context_compact/run_e2e.py
```
