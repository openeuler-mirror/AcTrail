# activity-anomaly 真实 Agent 回归

## 测试目标

验证真实 Agent 的请求增长、响应增长和超时命令能触发正确且可追溯的异常告警。

## 场景设计

用本地 provider 驱动真实 XiaoO 完成三轮 LLM 交互，依次执行短 Bash 命令、超过阈值的 Bash 命令并完成最终回复。以真实轨迹和命令耗时核对三类告警。

### 场景检查

该用例在隔离的 `PluginTestEnvironment` 中启动 `actraild`、`actrailweb` 和安装后的
`activity-anomaly` WIT component，再运行一个真实 xiaoO Agent。确定性的本地
OpenAI-compatible provider 驱动三轮 LLM 请求：先执行一条短 Bash 命令，再执行一条
超过阈值的 Bash 命令，最后完成回复。

测试严格验证：

- 真实轨迹至少包含三组完整且正确链接的 `llm.call/request/response`，并包含 provider
  指定的两次 Bash 工具调用，采集的最终响应包含 Agent 实际返回的完整文本；
- 恰好产生 `llm-request-growth`、`llm-response-growth` 和
  `command-duration-exceeded` 三类告警，每类一次且稳定后不重复；
- 告警 API 与 SQLite 持久化的类型和 payload 一致，每个 finding 都能关联到真实 action，
  LLM finding 还能关联回真实 `llm.call`；
- 插件保持 `active`、无 `last_error`，并且确实消费了 observation records。

测试使用 `scripts/install-release.sh` 安装的插件包。本地 provider 为
`tool_provider.py`，长命令脚本为
`examples/plugins/wit-component/activity-anomaly/long-running-command.sh`。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、root 权限和可工作的 eBPF 环境 | 启动采集与治理 |
| AcTrail | release 产物和测例使用的官方插件包 | 执行、保存和查询测试结果 |

需要真实 XiaoO、已安装的 activity-anomaly 插件及可启动的本地 provider。

## 测试步骤摘要

1. 刷新默认配置，启动 daemon、Web、本地 provider，并加载安装后的 activity-anomaly 插件。
2. 配置请求增长、响应增长和命令耗时阈值，保证长命令时长大于命令告警阈值。
3. 通过真实 XiaoO 完成三轮交互，依次执行短 Bash 命令和长 Bash 命令，保存 trace ID 与实际最终回复。
4. 按 trace ID 核对三组 LLM 请求与响应、两次工具调用，以及采集响应中的实际回复文本。
5. 核对三类告警各出现一次，比较 API 与 SQLite 的 payload，并将 finding 关联到实际 action 和 LLM 调用。
6. 确认插件保持 active 且无运行错误，停止服务并清理数据。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 真实轨迹 | 三轮交互完成 | 至少三组完整 LLM 关联，包含两次工具调用，最终响应与 Agent 实际回复一致 |
| 告警数量 | 请求、响应增长及长命令达到条件 | 三类告警各一次，稳定后无重复 |
| 证据与持久化 | 告警已生成 | API 与 SQLite payload 一致，finding 可关联到实际 action 和 LLM 调用 |
| 插件运行 | 场景完成 | 插件 active，无 last_error，确实消费观测记录 |

## 快速运行

从仓库根目录通过刷新默认配置的聚合入口运行：

```bash
sudo -E python3.11 tests/e2e/regression/test_all.py \
  --case plugin_activity_anomaly
```

也可单独运行：

```bash
sudo -E python3.11 tests/e2e/regression/activity_anomaly/run_e2e.py \
  --cleanup
```

可配置参数均使用环境变量：

- `ACTIVITY_ANOMALY_E2E_XIAOO_BINARY`：真实 xiaoO 可执行文件；
- `ACTIVITY_ANOMALY_E2E_PROVIDER_READY_TIMEOUT_SECONDS`：provider 就绪超时，默认 15 秒；
- `ACTIVITY_ANOMALY_E2E_ALERT_TIMEOUT_SECONDS`：轨迹和告警收敛超时，默认 20 秒；
- `ACTIVITY_ANOMALY_E2E_COMMAND_THRESHOLD_MS`：长命令告警阈值，默认 500 毫秒；
- `ACTIVITY_ANOMALY_E2E_LONG_COMMAND_SECONDS`：长命令时长，默认 2 秒；
- 通用的 `COMMAND_TIMEOUT_SECONDS`、`LAUNCH_TIMEOUT_SECONDS`、`DRAIN_ATTEMPTS` 和
  `DRAIN_INTERVAL_SECONDS` 使用 `ACTIVITY_ANOMALY_E2E_` 前缀。

长命令时长必须严格大于告警阈值，否则测试在启动阶段失败。
