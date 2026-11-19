# 告警转发端到端回归

## 测试目标

验证落库告警能经 daemon 和 alert proxy 转发给多个订阅者，并保持告警内容一致。

## 场景设计

启用 builtin forwarding，由 daemon 自动启动 proxy。两个 TCP subscriber 完成握手、订阅和心跳应答后，运行真实失败命令触发阈值为 1 的连续失败告警，并核对两端收到的内容。可用时再通过真实 Agent 触发告警。

### 场景检查

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

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、root 权限和可工作的 eBPF 环境 | 启动采集与治理 |
| AcTrail | release 产物和测例使用的官方插件包 | 执行、保存和查询测试结果 |

需要 alert proxy release 产物、官方连续失败插件及可绑定的本地 TCP 端口。

## 测试步骤摘要

1. 刷新默认配置并启用 builtin forwarding，启动 daemon，确认其自动拉起 alert proxy。
2. 加载连续失败告警插件，将阈值设为 1，并让两个 TCP subscriber 完成握手、订阅和心跳应答。
3. 通过 actrailctl launch 执行真实失败命令，记录 trace ID，查询落库告警。
4. 等待两个 subscriber 收到告警，将 trace ID、时间、UUID、类别、严重度、描述、labels 与 extras 逐项对照存储记录。
5. 存在可用工具型 Agent 时，再由真实 Agent 触发告警并核对转发结果。
6. 停止订阅者、proxy 与 daemon，清理测试数据。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 代理启动 | forwarding 已启用 | daemon 自动启动 alert proxy |
| 多订阅者 | 两个 subscriber 已订阅 | 同一存储告警到达两个 subscriber |
| 告警内容 | 告警经 UDS 与 broadcaster 转发 | trace ID、时间、UUID、类别、严重度、描述、labels 与 extras 一致 |
| 真实 Agent | 存在可用工具型 Agent | 实际工具失败产生可转发的告警 |

## 快速运行

运行单个测例：

```bash
sudo -E python3.11 tests/e2e/regression/alert_forwarding/run_e2e.py
```

保留失败现场：

```bash
sudo -E python3.11 tests/e2e/regression/test_all.py \
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
