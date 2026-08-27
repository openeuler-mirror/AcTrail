# 沙箱告警数据模型

> 本文说明沙箱资源告警的触发规则、持久化字段、外发映射和在线配置边界。

沙箱告警由 `sandbox-resource-alert` 从沙箱 observation 判定。它不做 trace 关联，不进入依赖 `TraceId` 的主告警表，也不把外部 JSON 编码职责带入检测过程。

## 告警判定

插件按 `(gateway_id, sb_id)` 保存有界来源状态，并用 `guest_boot_id` 区分 Guest boot。boot 标识变化时，内存越阈状态和 CPU baseline 都会重建。

| 告警 | 触发规则 | 重复规则 | 检测时间 |
| --- | --- | --- | --- |
| `OomKilled` | 收到一个 `OomVictimObservation` | 每个 victim observation 都产生一条 | `detected_at_ms` |
| `OomRisk` | `available_bytes` 从阈值以上进入阈值以下；比较条件为 `< threshold` | 持续低内存不重复；恢复后再次越界才产生下一条 | `sampled_at_ms` |
| `HighCpu` | 相邻累计 CPU 快照算出的利用率从阈值以下进入阈值以上；比较条件为 `>= threshold` | 持续高 CPU 不重复；恢复后再次越界才产生下一条 | `sampled_at_ms` |
| `HighRead` | 单个进程采样区间的 `read_bytes > threshold` | 每个超过阈值的区间都可产生一条 | `sample_ended_ms` |
| `HighWrite` | 单个进程采样区间的 `write_bytes > threshold` | 每个超过阈值的区间都可产生一条 | `sample_ended_ms` |

CPU 利用率使用整数 basis points，`10000` 表示 100%。首个快照、Guest boot 变化、累计计数倒退或总 tick 没有增量时，只建立新 baseline，不产生 `HighCpu`。

Guest 资源快照中的 `oom_kill_count` 不触发 `OomKilled`。所有 OOM victim 都生成 `critical` 告警；`monitored` 事件保留被观测谱系根，`unknown` 和 `unmonitored` 不伪造进程稳定标记。

## 结构化记录

进入 Sandbox Alert DB 的记录由以下部分组成：

| 层次 | 字段 | 说明 |
| --- | --- | --- |
| 来源 | `gateway_id: u32` | 当前 daemon 中的 live gateway connection ID，必须非零 |
| 来源 | `sb_id: u32` | 当前 gateway 中的 live SB connection ID，必须非零 |
| 批次位置 | `batch_sequence: u64` | 产生该告警的 observation batch sequence |
| 批次位置 | `observation_index: u32` | observation 在 batch 中的位置 |
| 告警主体 | `kind` | 下表所列的 typed alert 字段 |
| 持久化 | `alert_id: u64` | 数据库生成的告警 ID |
| 持久化 | `ingest_epoch: u64` | 独立 alert store 的 ingest epoch |
| 持久化 | `persisted_at_ms: u64` | 数据库提交时间，不替代检测时间 |

每种 `kind` 保存的业务字段如下：

| `kind` | 字段 |
| --- | --- |
| `HighCpu` | `guest_boot_id`、`sampled_at_ms`、`usage_basis_points`、`threshold_basis_points` |
| `OomKilled` | `guest_boot_id`、`detected_at_ms`、`victim_pid`、`victim_comm[16]`、`attribution`、可选 `monitored_root` |
| `OomRisk` | `guest_boot_id`、`sampled_at_ms`、`available_bytes`、`threshold_bytes` |
| `HighRead` | `guest_boot_id`、`process`、`sample_started_ms`、`sample_ended_ms`、`bytes`、`threshold_bytes` |
| `HighWrite` | `guest_boot_id`、`process`、`sample_started_ms`、`sample_ended_ms`、`bytes`、`threshold_bytes` |

`process` 和 `monitored_root` 都使用 `pid: u32`、`start_time_ticks: u64`、`executable_name: [u8; 16]` 的稳定进程标记。

## 外发映射

告警只有在 Sandbox Alert DB 事务提交成功后，才产生一份可丢弃的外发副本。该副本被标准化为 `ForwardAlert`：

| Sandbox alert | `category` | `severity` | `source.process` | `description` | `extras` 中的业务量 |
| --- | --- | --- | --- | --- | --- |
| `OomKilled` | `sandbox.resource.oom_killed` | `critical` | 仅 `monitored` 时使用 `monitored_root` | `Sandbox kernel selected an OOM victim` | `victim_pid`、`victim_comm`、`attribution` |
| `OomRisk` | `sandbox.resource.oom_risk` | `warning` | 省略 | `Sandbox available memory crossed threshold` | `available_bytes`、`threshold_bytes` |
| `HighCpu` | `sandbox.resource.high_cpu` | `warning` | 省略 | `Sandbox CPU usage crossed threshold` | `usage_basis_points`、`threshold_basis_points` |
| `HighRead` | `sandbox.process.high_read` | `warning` | 使用被观测根进程标记 | `Sandbox process read bytes crossed threshold` | `sample_started_ms`、`bytes`、`threshold_bytes` |
| `HighWrite` | `sandbox.process.high_write` | `warning` | 使用被观测根进程标记 | `Sandbox process write bytes crossed threshold` | `sample_started_ms`、`bytes`、`threshold_bytes` |

所有类型的 `extras` 还带有 `batch_sequence` 和 `observation_index`。`extras` 不重复保存 gateway、SB、boot 或进程身份。

Sandbox source 的公共部分是：

```text
gateway_id           u32，非零
sb_id                u32，非零
guest_boot_id        16 raw bytes，非零
process              optional ProcessMarker
```

它不包含 `trid`。对外 JSON 中，`guest_boot_id` 表示为 UUID 字符串，`executable_name` 使用 16 bytes 的无损十六进制表示；资源类告警省略 `process`。daemon 到 alert proxy 的完整二进制字段顺序与接收校验见 [Alert proxy 协议](../protocols/alert-proxy.md)。

外发类别按 category filter 精确匹配。forwarder 未启用、类别不匹配、队列已满或连接断开时，只丢弃本次外发副本；已经提交的数据库记录保持不变。

## 配置与在线更新

`SandboxResourceAlertConfig` 只拥有检测配置：

| 字段 | 类型 | 启动校验 | 可在线修改 |
| --- | --- | --- | --- |
| `cpu_usage_threshold_basis_points` | `u16` | `1..=10000` | 是 |
| `memory_available_threshold_bytes` | `u64` | 必须大于 0 | 是 |
| `read_interval_threshold_bytes` | `u64` | 必须大于 0 | 是 |
| `write_interval_threshold_bytes` | `u64` | 必须大于 0 | 是 |
| `source_state_capacity` | `u32` | 必须大于 0 且能转换为平台 `usize` | 否，只能在插件加载前设置 |

Web plugin config API 接收更新时执行 JSON、未知字段和类型校验，并完成上述值域检查。有效更新先原子替换配置文件，再发布不可变运行时快照；校验或持久化失败时继续使用旧配置。

插件 worker 每个 observation batch 只读取一次快照，因此同一 batch 不会混用新旧阈值。更新阈值不卸载插件、不更换 consumer，也不清空来源状态。`source_state_capacity` 决定状态表容量，在线请求不能改变它。

告警数据库路径、schema、writer queue、事务批量和保留策略属于独立的 `SandboxAlertsConfig`；proxy 连接和外发选择属于 `AlertForwardingConfig`。修改检测阈值不改变采集、传输、持久化或转发配置。

## 故障边界

Sandbox alert store 使用独立数据库文件、schema、连接、writer queue 和生命周期，不属于 AcTrail 主 Storage，也不属于无插件消费时使用的 Sandbox Evidence DB。

启动时，检测配置、数据库目录、SQLite、schema 和 writer 任一必要条件无效都会阻止该能力启动。运行期间，告警 admission、数据库事务、forwarding queue、proxy 或 subscriber 的故障分别局限于当前操作；它们不会反向形成 observation consume 错误，也不会阻塞 gateway 或 Guest 采集。
