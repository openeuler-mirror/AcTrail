# 沙箱观测数据模型

> 本文给出执行隔离通路中三类沙箱观测及其批次编码、字段语义和接收校验边界。

沙箱观测描述 Guest 内部的进程 I/O、整机资源和 OOM victim。它不包含文件或网络内容、系统调用参数、脑侧进程身份、trace ID 或 semantic action。

## 公共标识与计量

| 名称 | 表示 | 约束 |
| --- | --- | --- |
| `guest_boot_id` | 本次 Guest boot 的标识 | 固定 16 bytes；用于区分重启前后的累计量和进程标记 |
| `ProcessMarker` | 一个根进程谱系 | `pid: u32`、`start_time_ticks: u64`、`executable_name: [u8; 16]` |
| `executable_name` | 根发现时精确匹配的 Linux `comm` | 保留原始 16 bytes，不按 UTF-8 文本重编码 |
| `*_ms` | Guest 产生的检测或采样时间 | `u64` 毫秒值；接收和持久化时间不覆盖它 |
| `*_bytes` | 字节数 | 无符号累计量或采样区间增量，具体含义由字段说明决定 |
| `*_ticks` | Guest 内核累计 tick 或进程启动 tick | 不换算为墙钟时间 |

根进程谱系包含匹配根及其通过 `fork`、`vfork` 或 `clone` 创建的后代。后代在 `exec` 后仍属于原谱系；进程退出后从活跃集合移除。`pid`、`start_time_ticks` 和发现时的 `comm` 共同防止 PID 复用造成错误归属。

## `ProcessIoCounters`

`ProcessIoCounters` 表示一个采样区间内、一个根进程谱系的聚合增量。

| 顺序 | 字段 | Wire 类型 | 语义 |
| ---: | --- | --- | --- |
| 1 | `guest_boot_id` | 16 bytes | Guest boot 标识 |
| 2 | `process.pid` | `u32` | 根进程 PID |
| 3 | `process.start_time_ticks` | `u64` | 根进程启动 tick |
| 4 | `process.executable_name` | 16 bytes | 根发现时的原始 `comm` |
| 5 | `sample_started_ms` | `u64` | 采样区间起点 |
| 6 | `sample_ended_ms` | `u64` | 采样区间终点，也是 I/O 告警的检测时间 |
| 7 | `read_operations` | `u64` | 成功完成的 `read(2)` 次数 |
| 8 | `read_bytes` | `u64` | 成功 `read(2)` 实际返回字节数之和 |
| 9 | `write_operations` | `u64` | 成功完成的 `write(2)` 次数 |
| 10 | `write_bytes` | `u64` | 成功 `write(2)` 实际返回字节数之和 |
| 11 | `failed_read_operations` | `u64` | 返回负值的 `read(2)` 次数 |
| 12 | `failed_write_operations` | `u64` | 返回负值的 `write(2)` 次数 |

失败调用只增加失败次数，不增加成功次数或成功字节数。采集器在内核侧聚合同一谱系的计数，不复制用户缓冲区，也不逐次发送系统调用事件。

Wire body 固定为 108 bytes，类型 code 为 `1`。

## `GuestResourceSnapshot`

`GuestResourceSnapshot` 描述一个采样时刻的整个 Guest，不依赖目标进程是否存在。

| 顺序 | 字段 | Wire 类型 | 语义 |
| ---: | --- | --- | --- |
| 1 | `guest_boot_id` | 16 bytes | Guest boot 标识 |
| 2 | `sampled_at_ms` | `u64` | 资源采样时间，也是资源告警的检测时间 |
| 3 | `cpu.total_ticks` | `u64` | CPU 累计总 tick |
| 4 | `cpu.idle_ticks` | `u64` | CPU 累计空闲 tick |
| 5 | `cpu.logical_cpu_count` | `u16` | Guest 逻辑 CPU 数量 |
| 6 | `memory.total_bytes` | `u64` | Guest 总内存 |
| 7 | `memory.available_bytes` | `u64` | Guest 当前可用内存 |
| 8 | `memory.used_bytes` | `u64` | Guest 当前已用内存 |
| 9 | `memory.oom_kill_count` | `u64` | `vmstat` 中 `oom_kill` 的累计值 |

CPU 利用率由相邻快照的 `total_ticks` 和 `idle_ticks` 增量计算。`oom_kill_count` 仅是累计资源指标，不生成 `OomKilled`；具体 victim 由 `OomVictimObservation` 表示。

Wire body 固定为 74 bytes，类型 code 为 `2`。

## `OomVictimObservation`

`OomVictimObservation` 表示 Guest 内核选中的一个 OOM victim。

| 顺序 | 字段 | Wire 类型 | 语义 |
| ---: | --- | --- | --- |
| 1 | `guest_boot_id` | 16 bytes | Guest boot 标识 |
| 2 | `detected_at_ms` | `u64` | Guest boot 单调时间换算出的检测时间 |
| 3 | `victim_pid` | `u32` | victim PID |
| 4 | `victim_comm` | 16 bytes | 内核提供的原始 `comm` |
| 5 | `attribution` | `u8` | `0=unknown`、`1=monitored`、`2=unmonitored` |
| 6 | `monitored_root.pid` | `u32` | 被观测谱系根 PID；无根标记时为 0 |
| 7 | `monitored_root.start_time_ticks` | `u64` | 被观测谱系根启动 tick；无根标记时为 0 |
| 8 | `monitored_root.executable_name` | 16 bytes | 被观测谱系根 `comm`；无根标记时全 0 |
| 9 | reserved | 4 bytes | 必须全 0 |

归因与根标记必须成对成立：

| `attribution` | `monitored_root` | 含义 |
| --- | --- | --- |
| `monitored` | 必须存在 | 事件发生时 victim 命中 Guest eBPF 谱系跟踪表 |
| `unknown` | 必须不存在 | 未命中跟踪表，不能证明 victim 不属于监控范围 |
| `unmonitored` | 必须不存在 | 采集器能够证明 victim 已被完整分类且不在监控范围 |

当前采集器命中时输出 `monitored`，未命中时输出 `unknown`。它不会把谱系发现窗口或跟踪容量不足误报为 `unmonitored`。

Wire body 固定为 77 bytes，类型 code 为 `3`。未知归因值、非零 reserved bytes，或归因与根标记不一致都会使当前 frame 校验失败。

## `ObservationBatch` 编码

一个 batch 由固定头部和零个或多个带长度的 observation 组成。所有整数使用大端序。

```text
sequence          u64
observation_count u16
repeat observation_count times:
  type_code       u8
  body_length     u16
  body            body_length bytes
```

`sequence` 在每个新 SB sender session 中从 1 开始。它只表达该 session 内的顺序；重连会建立新的 session ID、baseline 和 sequence 边界，不能据此推断跨连接连续性。

| 校验阶段 | 接收端要求 |
| --- | --- |
| Frame header | 校验 magic `0xac71`、版本 `1`、消息 code 和 payload 长度；完整 frame 不得超过 256 KiB |
| Batch header | payload 至少容纳 `sequence` 与 `observation_count`；计数不得超过 `u16` |
| Observation envelope | type code 必须是 `1`、`2` 或 `3`，body 长度必须与该类型固定长度完全一致 |
| Observation body | 按固定字段宽度读取；OOM 额外校验归因、根标记与 reserved bytes |
| Batch boundary | 读取声明数量后不得有 trailing bytes；半帧会等待补齐，截断 payload 校验失败 |

接收端在分配 payload 前完成 frame 长度校验。gateway 只恢复 frame 边界并转发未经修改的 SB frame；daemon 负责确认内层恰好包含一个有效 `ObservationBatch`，再以当前连接的 `(gateway-id, sb-id)` 标记来源。

## 发布与路由边界

只有完成 hello/welcome 且当前 session 有效时，采集结果才进入有界发送队列。断连或重连期间继续采集，但 observation 立即丢弃，不缓存、不持久化、不补发；进程 I/O baseline 仍持续推进。

daemon 对每条 observation 的一次成功 interest query 只选择一条路由：有匹配插件时投递给所有匹配插件；没有匹配插件时写入独立 Sandbox Evidence DB。查询失败、插件投递失败或 evidence 写入失败不会触发另一条补偿路由，也不会进入 AcTrail 主 Storage。
