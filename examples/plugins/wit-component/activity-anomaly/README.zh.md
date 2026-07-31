# 请求/响应增长与长命令告警插件

`actrail.activity-anomaly` 是 WIT Component 观测插件。相关活动事实一旦完整且命中规则，插件立即写入告警，不等待 Agent 或 trace 结束：

| 告警类型（`kind`） | 触发条件 |
| --- | --- |
| `llm.request.growth` | LLM 请求达到固定阈值，或相对历史基线异常增长 |
| `llm.response.growth` | LLM 响应达到固定阈值，或相对历史基线异常增长 |
| `command.duration.exceeded` | Agent 顶层命令耗时超过阈值 |

配置、加载、更新和验证步骤见[插件使用手册](../../../../docs/plugins/activity-anomaly-manual.zh.md)。

## 检测规则

请求和响应分别建立 trace 内滚动基线，分组键为：

```text
(process_id, model, server_address, url_path)
```

满足以下任一条件时产生增长告警：

1. 当前字节数达到 `hard_limit_bytes`；
2. 历史样本数达到 `minimum_samples`，且当前值同时满足最小值、最小增量和增长倍数要求。

基线仅使用完整请求或响应的历史中位数。不同 trace、进程、模型、服务端或 URL 不共享基线。

长命令规则仅处理具有 Agent 归属的顶层 `command.invocation`。判断条件为：

```text
(ended_at_ms 或当前宿主时间) - started_at_ms > maximum_duration_ms
```

命令仍在运行且尚未达到阈值时，插件申请在 `started_at_ms + maximum_duration_ms + 1` 复评；跨过阈值后立即提交告警，不等待命令结束。运行态 finding 的 `status` 为 `in_progress`、`ended_at_ms` 为 `null`，`observed_at_ms` 记录命中时间。启用 seccomp notify 并且 argv 已完成投影时，告警包含完整命令行；否则实时告警回退为仅包含可执行文件。

## 权限与数据范围

插件需要以下 capability：

| capability | 用途 |
| --- | --- |
| `trace-activity-read` | 在当前 observation trace 范围内读取已持久化的 LLM 字节计数、命令执行事实和容器归属 |
| `alert-write` | 写入 manifest 中已声明的告警 |

插件不读取请求或响应正文，也不能查询其他 trace。实时分析、定时复评和告警写入位于异步 observation worker，不在被观测进程的同步执行路径上。trace 进入终态后还会执行一次兜底分析，并释放该 trace 的插件状态。

## 多容器隔离

插件状态按 `trace_id` 隔离，并使用宿主侧进程身份区分容器内相同 PID。告警 payload 包含 `root_container_id` 和 `root_process_id`，不同容器不会共享增长检测窗口。

## 构建

在仓库根目录执行：

```bash
cargo fmt --all
cargo build --release
cargo build --release --target wasm32-wasip2 \
  --manifest-path examples/plugins/wit-component/activity-anomaly/Cargo.toml
```

release 安装脚本会安装插件文件，但不会自动加载插件：

```bash
scripts/install-release.sh
```

## 配置与输出

- 默认配置：[activity-anomaly.config.json](activity-anomaly.config.json)
- 配置约束：[activity-anomaly.config.v1.schema.json](activity-anomaly.config.v1.schema.json)
- LLM 增长告警结构：[llm-growth.payload.v1.schema.json](llm-growth.payload.v1.schema.json)
- 长命令告警结构：[command-duration.payload.v1.schema.json](command-duration.payload.v1.schema.json)
- 插件清单：[activity-anomaly.plugin.toml](activity-anomaly.plugin.toml)

每种告警在一个 trace 中最多保留一条，多个命中项存放在 `findings` 中。插件为每类告警提交稳定幂等键，因此并发插件实例、延迟 observation 和终态兜底不会落入重复记录。超过 `finding_max_count` 的数量记录在 `truncated_count`。

## 验证

真实 xiaoO 多容器验证见[端到端测试说明](../../../../tests/agent-trace/multi-container-activity-anomaly/README.md)。

仓库同时提供可直接运行的长命令示例：

```bash
bash examples/plugins/wit-component/activity-anomaly/long-running-command.sh 60 5
```

第一个参数是目标阈值秒数，第二个参数是超过阈值后继续运行的秒数；上述命令共运行 65 秒。真实 Agent E2E 会把阈值调为 500ms，并让 xiaoO 的 bash 工具执行同一脚本，以便在脚本结束前验证告警已上报。
