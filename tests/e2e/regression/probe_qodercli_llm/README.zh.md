# 回归测试：Qoder CLI LLM 交互采集

## 测试目标

验证 AcTrail 能通过 Qoder LLM 编解码插件采集 Qoder CLI 的模型请求和响应，并将它们关联到对应的 LLM 调用。

## 场景设计

使用刷新后的默认采集配置启动 AcTrail，并加载 Qoder LLM 编解码插件。通过 AcTrail 启动 Qoder CLI 流式会话，执行一次开放式文本任务。以 Qoder CLI 实际返回的助手文本核对插件解码后的响应正文。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、可工作的 eBPF 环境、root 权限 | 采集 Qoder CLI 活动 |
| AcTrail | 可运行的 release 产物 `actraild`、`actrailctl`、`actrailviewer` | 启动采集、保存并查询观测记录 |
| Qoder CLI | 支持流式 JSON 输入和输出的 CLI | 执行文本任务并返回实际回复 |
| Qoder LLM 编解码插件 | 仓库内的[插件清单](../../../../examples/plugins/wasm-legacy/llm-codec-qoder/plugin.toml)和同目录的 WASM 产物 | 解码 Qoder 模型请求和流式响应 |
| 模型服务 | Qoder CLI 已配置认证，服务可访问 | 执行真实的模型请求 |

## 测试步骤摘要

1. **检查 Qoder CLI 可用性。** 直接运行 Qoder CLI 完成一次模型请求。Qoder CLI 缺失或可用性检查失败时，用例标记为 `SKIPPED`。
2. **准备观测环境。** 在测例工作目录用 `actrailctl init` 生成默认配置。用 `actraild start` 启动守护进程。
3. **加载 Qoder LLM 编解码插件。** 从仓库内的插件清单加载实例 `qoder.llm-codec`。
4. **启动 Qoder CLI 会话。** 通过 `actrailctl launch` 启动流式会话，关闭工具和会话保存。记录启动结果中的 trace ID。
5. **执行文本任务。** 提交一个简短的开放式问题。等待任务成功结束，从最终的 `result` 事件取得实际回复文本。
6. **完成数据采集。** 结束 Qoder CLI 会话后停止守护进程，等待观测记录落盘。
7. **核对观测记录。** 使用 `actrailviewer` 查询第 4 步取得的 trace ID。

   | 检查项 | 条件 | 预期结果 |
   | --- | --- | --- |
   | Trace 终态 | 守护进程已停止 | trace 状态为 `Completed` 或 `Exited`，健康状态为 `Clean` |
   | 调用关联 | trace 下的 LLM 请求和响应 | 每条 `llm.call` 关联一条 `llm.request`，最多关联一条 `llm.response`。每条请求或响应仅关联到一条调用。无响应的调用保持 `unknown/partial`，无结束时间 |
   | 任务内容 | Qoder CLI 成功完成任务并返回文本 | 至少一条 `success/complete` 响应的正文包含实际回复全文。同一调用的请求保留插件解码后的规范化内容 |
   | 采集收尾 | 守护进程已停止 | LLM 请求、响应和 SSE 流处于终态 |

## 快速运行

### 测试所用环境变量

| 环境变量 | 未设置时测试框架的行为 | 用途 |
| --- | --- | --- |
| `QODERCLI_E2E_COMMAND_TIMEOUT_SECONDS` | 等待 30 秒 | AcTrail 命令的等待时限 |
| `QODERCLI_E2E_LAUNCH_TIMEOUT_SECONDS` | 等待 180 秒 | Qoder CLI 文本任务的等待时限 |
| `QODERCLI_E2E_BINARY` | 自动发现 Qoder CLI 可执行文件 | 指定 Qoder CLI 路径 |
| `ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS` | 等待 60 秒 | Qoder CLI 可用性检查的等待时限 |

### 运行指令

从仓库根目录执行以下命令。

```bash
sudo -E env "HOME=$HOME" "PATH=$PATH" \
  python3 tests/e2e/regression/probe_qodercli_llm/run_e2e.py
```
