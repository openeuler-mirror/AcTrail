# 启用 activity-anomaly

> 本文指导插件管理员加载异常活动插件、配置正式阈值，并通过插件状态和告警结果定位问题。

`actrail.activity-anomaly` 是运行在 WebAssembly component 中的异步观测插件，用于检测
LLM 请求/响应增长和 Agent 顶层长命令。它需要读取 trace 活动事实并写入告警，因此必须显式授予
`trace-activity-read` 和 `alert-write`。

## 配置

正式插件包包含：

```text
activity-anomaly.plugin.toml
activity-anomaly.config.json
activity-anomaly.config.v1.schema.json
actrail_activity_anomaly_plugin.wasm
```

主要配置项如下：

| 配置项 | 含义 |
| --- | --- |
| `enabled` | 是否启用对应规则 |
| `hard_limit_bytes` | 请求或响应的固定字节阈值 |
| `window_size` / `minimum_samples` | 历史窗口及启动相对判断所需样本数 |
| `ratio_per_mille` | 相对增长倍数，`2000` 表示 2 倍 |
| `minimum_growth_bytes` / `minimum_current_bytes` | 相对增长的最小增量和最小当前值 |
| `maximum_duration_ms` | 顶层命令时长阈值 |

默认配置以 2 MiB 请求、4 MiB 响应和 60 秒顶层命令作为固定阈值，并结合历史中位数
判断增长。随 release 安装的配置是阈值调整起点；字段缺失、类型错误或值非法会让加载失败，
不会回退到隐式阈值。插件只在加载时读取配置。

## 加载

```bash
sudo target/release/actraild --config operator.conf plugin load \
  --manifest /absolute/path/activity-anomaly.plugin.toml \
  --plugin-config /absolute/path/activity-anomaly.config.json \
  --grant trace-activity-read \
  --grant alert-write \
  --instance actrail.activity-anomaly \
  --persist

sudo target/release/actraild --config operator.conf plugin status \
  --instance actrail.activity-anomaly
```

固定部署可按 [管理插件](manage.md#固定启动清单) 改用 `[plugins.startup]`；同一实例不得
同时使用 `--persist`。

## 行为与更新

- 完整 LLM 请求或响应命中规则后立即提交告警，不等待 trace 结束。
- 运行中的顶层命令达到阈值后由 observation worker 定期复评并提交告警。
- 运行态长命令 finding 使用 `status=in_progress`、`ended_at_ms=null`，命中时间记录在 `observed_at_ms`。
- 每个 trace 的每类告警使用稳定幂等键；终态分析只补充实时阶段尚未成功提交的告警。
- 实时阶段尚无 argv 时，长命令告警只显示 executable。
- 增长窗口按 trace、进程、模型、服务端和 URL 隔离；不同容器不共享窗口。

插件管理员更新阈值、WASM 或 manifest 时，需要先卸载，再加载并检查状态。卸载期间未结束的 trace 不会
由新实例补充分析。

## 排错

| 现象 | 检查 |
| --- | --- |
| 加载失败 | `plugin status` 的 `last_error`；配置必须通过包内 schema |
| 没有告警 | 两项 grants、对应规则的 `enabled`、活动是否达到正式阈值 |
| 告警只有 executable | argv 投影在实时命中时尚未完成；终态结果可能补足 |
| 重启后配置未更新 | 启动清单实例需要重启 daemon；持久化实例需要卸载后重新加载 |
