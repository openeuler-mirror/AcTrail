# 告警转发端到端回归

该测例使用刷新后的 release 产物和隔离配置启动真实 `actraild`。

`actraild` 根据启用的 builtin forwarding 配置自动拉起
`actraild-alert-proxy`，随后加载官方 `tool-consecutive-failure-alert` WASM
插件。插件的连续失败阈值配置为 `1`。

两个真实 TCP subscriber 分别完成 v1 握手、订阅和心跳应答。
测例通过 `actrailctl launch` 执行 `assets/trigger-alert.sh`，由脚本运行一个真实失败
命令。告警必须先写入主 SQLite Storage，再经过 daemon UDS、proxy broadcaster
到达两个 subscriber。

外发断言包括 trace ID、检测时间、UUID、类别、严重度、描述、labels 和 extras。
测试还会在存在可用工具型 agent 时运行一次真实 agent 告警回合。

运行单个测例：

```bash
sudo -E python3.11 tests/v2/regression/alert_forwarding/run_e2e.py
```

保留失败现场：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case alert_forwarding \
  --fail-fast \
  --no-cleanup
```

可配置入口：

- `ALERT_FORWARDING_E2E_SUBSCRIBER_PORT`
- `ALERT_FORWARDING_E2E_ALERT_TIMEOUT_SECONDS`
- `ALERT_FORWARDING_E2E_NEGATIVE_WINDOW_SECONDS`
- `ALERT_FORWARDING_E2E_COMMAND_TIMEOUT_SECONDS`
- `ALERT_FORWARDING_E2E_LAUNCH_TIMEOUT_SECONDS`
