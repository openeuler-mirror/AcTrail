# LLM 会话异常告警插件

`actrail.llm-turn-anomaly` 是 WIT Component 观测插件。插件订阅语义动作与 trace 生命周期，在每次收到包含 LLM 活动的 observation batch 时对当前 trace 已持久化的 LLM 会话记录实时复评，一旦命中规则立即写入告警，不等待 Agent 或 trace 结束：

| 告警类型（`kind`） | 触发条件 |
| --- | --- |
| `llm.turn.high_frequency` | 分组在滑动时间窗口内请求次数达到阈值 |
| `llm.turn.consecutive_retry` | 分组内连续失败的请求数达到阈值 |
| `llm.turn.repeated_similar` | 分组内窗口中出现重复相似请求 |
| `llm.turn.error_ratio` | 分组内请求错误率超过阈值 |
| `llm.turn.context_growth` | 分组内请求体字节数相对滚动基线异常膨胀 |

配置、加载、更新和验证步骤见[插件操作手册](../../../../docs/plugins/operator-manual.zh.md)。

## 检测规则

所有规则按 `(process_id, model)` 分组，使用 trace 内已持久化的 LLM 会话记录（请求与响应、字节计数、开始时间），分组间不共享任何状态或基线。

### 高频请求

对分组内按 `started_at` 排序的请求维护 `window_size_ms` 滑动窗口。分组请求总数达到 `min_exchanges` 后，窗口中任一请求数达到 `threshold` 的区间都会产生一条 finding，记录 `exchange_count`、`window_start_ms` 和 `window_end_ms`。

### 连续重试

对分组内请求按时间顺序扫描，`response_complete` 为 `false`（即失败）且 `request_body_bytes >= min_request_bytes` 的请求计入连续段；非失败请求或请求体过小的请求会中断连续段。连续段长度达到 `consecutive_count` 时产生一条 finding，记录段首尾的 `request_action_id` 与开始时间。

### 重复相似请求

对分组内请求按时间顺序扫描，在每个长度 `similarity_window` 的连续窗口中找最长的“相似”请求串。两个请求体字节数满足

```text
|a_bytes - b_bytes| * 1000 <= max(a_bytes, b_bytes) * similarity_tolerance_ratio_per_mille
```

即视为相似；完全相同也视为相似。最长相似串达到 `min_repeat_count` 时产生一条 finding，记录代表请求的 `request_action_id`、`repeat_count` 和字节数。

### 错误率

对分组内请求统计 `response_complete` 为 `false` 的比例（千分比）。请求总数达到 `minimum_exchanges` 且实际错误率不低于 `error_ratio_per_mille` 时产生一条 finding，记录 `total_exchanges`、`error_count` 和 `actual_ratio_per_mille`。

### 上下文快速膨胀

对分组内请求维护最近 `window_size` 个请求体字节数的滚动历史；历史样本数达到 `minimum_samples` 后取其中位数作为基线。当前请求同时满足以下条件时产生一条 finding：

```text
bytes >= minimum_growth_bytes
基线中位数 >= minimum_baseline_bytes
bytes - 基线 >= minimum_growth_bytes
bytes * 1000 >= 基线 * growth_ratio_per_mille
```

基线仅来自本 trace、本分组的历史中位数。不同 trace、进程或模型不共享基线。

若分组尚未持久化任何请求，插件会申请在 250ms 后复评。命中告警后 `findings` 记录命中时的上下文信息，`observed_at` 与告警写入时间由宿主记录。

## 权限与数据范围

插件需要以下 capability：

| capability | 用途 |
| --- | --- |
| `trace-activity-read` | 在当前 observation trace 范围内分页读取已持久化的 LLM 会话记录 |
| `alert-write` | 写入 manifest 中已声明的告警 |

插件不读取请求或响应正文，也不能查询其他 trace。实时分析、定时复评和告警写入位于异步 observation worker，不在被观测进程的同步执行路径上。trace 进入终态后插件执行一次兜底分析并释放该 trace 的插件状态。

## 多容器隔离

插件状态按 `trace_id` 隔离，并使用宿主侧进程身份区分容器内相同 PID。告警 payload 包含 `root_container_id` 和 `root_process_id`，不同容器不会共享检测窗口。

## 构建

在仓库根目录执行：

```bash
cargo fmt --all
cargo build --release
cargo build --release --target wasm32-wasip2 \
  --manifest-path examples/plugins/wit-component/llm-turn-anomaly/Cargo.toml
```

release 安装脚本会安装插件文件，但不会自动加载插件：

```bash
scripts/install-release.sh
```

## 配置与输出

- 默认配置：[llm-turn-anomaly.config.json](llm-turn-anomaly.config.json)
- 配置约束：[llm-turn-anomaly.config.v1.schema.json](llm-turn-anomaly.config.v1.schema.json)
- 高频请求告警结构：[llm-turn-anomaly.frequency.payload.v1.schema.json](llm-turn-anomaly.frequency.payload.v1.schema.json)
- 连续重试告警结构：[llm-turn-anomaly.consecutive-retry.payload.v1.schema.json](llm-turn-anomaly.consecutive-retry.payload.v1.schema.json)
- 重复相似请求告警结构：[llm-turn-anomaly.repeated-similar.payload.v1.schema.json](llm-turn-anomaly.repeated-similar.payload.v1.schema.json)
- 错误率告警结构：[llm-turn-anomaly.error-ratio.payload.v1.schema.json](llm-turn-anomaly.error-ratio.payload.v1.schema.json)
- 上下文膨胀告警结构：[llm-turn-anomaly.context-growth.payload.v1.schema.json](llm-turn-anomaly.context-growth.payload.v1.schema.json)
- 插件清单：[llm-turn-anomaly.plugin.toml](llm-turn-anomaly.plugin.toml)

每种告警在一个 trace 中最多保留一条，多个命中项存放在 `findings` 中。超过 `finding_max_count` 的数量记录在 `truncated_count`。`high_frequency.window_size_ms`、`threshold`、`min_exchanges`，`consecutive_retry.consecutive_count`，`repeated_similar.similarity_window`、`min_repeat_count`，`error_ratio.minimum_exchanges`、`error_ratio_per_mille`，`context_growth.window_size`、`minimum_samples` 以及 `page_size`、`trace_state_max_count`、`finding_max_count` 均受配置约束限制，非法取值会导致插件加载失败。
