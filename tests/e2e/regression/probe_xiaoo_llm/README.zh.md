# 回归测试：XiaoO LLM 交互采集

## 测试目标

验证 AcTrail 能采集 XiaoO 的模型请求和响应，并将它们关联到对应的 LLM 调用。

## 场景设计

在隔离的 AcTrail 环境中启动 XiaoO，使用刷新后的默认采集配置。XiaoO 沿用自身的模型和服务配置，关闭工具并执行一次开放式文本任务。以 XiaoO 实际返回的助手文本核对采集响应正文。

使用 HTTPS 和 rustls 时，AcTrail 自动查找 TLS 探针点并采集明文。使用明文 HTTP 时，AcTrail 通过 socket 采集。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、可工作的 eBPF 环境、root 权限 | 采集 XiaoO 活动 |
| AcTrail | 可运行的 release 产物 `actraild`、`actrailctl`、`actrailviewer` | 启动采集、保存并查询观测记录 |
| XiaoO | 支持非交互运行和 JSON 结果输出的 CLI | 执行文本任务并返回实际回复 |
| 模型服务 | XiaoO 已配置默认模型和认证，服务可访问 | 执行真实的模型请求 |

## 测试步骤摘要

1. **检查 XiaoO 可用性。** 直接运行 XiaoO 完成一次模型请求。XiaoO 缺失或可用性检查失败时，用例标记为 `SKIPPED`。
2. **准备观测环境。** 在测例工作目录用 `actrailctl init` 生成默认配置。用 `actraild start` 启动守护进程。
3. **启动 XiaoO。** 通过 `actrailctl launch` 启动 XiaoO，关闭工具并限制为一轮。记录启动结果中的 trace ID。
4. **执行文本任务。** 通过标准输入提交一个简短的开放式问题。等待任务成功结束，从 JSON 响应的 `raw_reply` 字段取得实际回复文本。
5. **完成数据采集。** XiaoO 退出后停止守护进程，等待观测记录落盘。
6. **核对观测记录。** 使用 `actrailviewer` 查询第 3 步取得的 trace ID。

   | 检查项 | 条件 | 预期结果 |
   | --- | --- | --- |
   | Trace 终态 | 守护进程已停止 | trace 状态为 `Completed` 或 `Exited`，健康状态为 `Clean` |
   | 调用关联 | trace 下的 LLM 请求和响应 | 每条 `llm.call` 关联一条 `llm.request`，最多关联一条 `llm.response`。每条请求或响应仅关联到一条调用 |
   | 调用状态 | 调用关联已建立 | 已配对的调用为 `unknown/inferred`，仅有请求的调用为 `unknown/partial`。两种调用均无结束时间 |
   | 任务内容 | XiaoO 成功完成任务并返回文本 | 至少一条 `success/complete` 响应的正文包含实际回复全文。同一调用的请求保留规范化内容，或在采集受限时保留有效的请求元数据 |
   | 采集收尾 | 守护进程已停止 | LLM 请求、响应和 SSE 流处于终态 |

## 快速运行

### 测试所用环境变量

| 环境变量 | 未设置时测试框架的行为 | 用途 |
| --- | --- | --- |
| `XIAOO_E2E_COMMAND_TIMEOUT_SECONDS` | 等待 30 秒 | AcTrail 命令的等待时限 |
| `XIAOO_E2E_LAUNCH_TIMEOUT_SECONDS` | 等待 180 秒 | XiaoO 文本任务的等待时限 |
| `XIAOO_E2E_BINARY` | 自动发现 XiaoO 可执行文件 | 指定 XiaoO CLI 路径 |
| `ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS` | 等待 60 秒 | XiaoO 可用性检查的等待时限 |

### 运行指令

从仓库根目录执行以下命令。

```bash
sudo -E python3 tests/e2e/regression/probe_xiaoo_llm/run_e2e.py
```
