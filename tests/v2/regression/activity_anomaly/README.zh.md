# activity-anomaly 真实 Agent 回归

该用例在隔离的 `PluginTestEnvironment` 中启动 `actraild`、`actrailweb` 和安装后的
`activity-anomaly` WIT component，再运行一个真实 xiaoO Agent。确定性的本地
OpenAI-compatible provider 驱动三轮 LLM 请求：先执行一条短 Bash 命令，再执行一条
超过阈值的 Bash 命令，最后返回唯一 marker。

测试严格验证：

- 真实轨迹至少包含三组完整且正确链接的 `llm.call/request/response`，并包含 provider
  指定的两次 Bash 工具调用及最终 marker；
- 恰好产生 `llm-request-growth`、`llm-response-growth` 和
  `command-duration-exceeded` 三类告警，每类一次且稳定后不重复；
- 告警 API 与 SQLite 持久化的类型和 payload 一致，每个 finding 都能关联到真实 action，
  LLM finding 还能关联回真实 `llm.call`；
- 插件保持 `active`、无 `last_error`，并且确实消费了 observation records。

测试使用 `scripts/install-release.sh` 安装到 `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}`
的插件包，不直接使用仓库内构建产物。provider 和长命令脚本复用既有重型 E2E 资产，
但该用例只启动一个宿主 Agent，不需要 Docker。

从仓库根目录通过刷新默认配置的聚合入口运行：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case plugin_activity_anomaly
```

也可单独运行：

```bash
sudo -E python3.11 tests/v2/regression/activity_anomaly/run_e2e.py \
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
