# Sandbox 资源告警通路

## 1. 职责边界

`sandbox-resource-alert` 只从 Hand observation 判定资源告警。
它不执行 trace 关联，不访问主 Storage，不编码外部 JSON，也不感知 `actraild-alert-proxy` 的连接结构。
daemon 同一时刻只允许一个活动的 `sandbox-resource-alert` 实例，避免同一 observation 被不同阈值实例重复判定和争用同一持久化身份。

```text
Guest resource / process I/O / OOM victim observation
  -> sandbox-resource-alert
  -> bounded SandboxAlert admission
  -> sandbox alert SQLite transaction
  -> builtin alert-forwarding plugin
  -> actraild-alert-proxy
  -> matching external subscribers
```

Sandbox 告警使用独立的数据库文件、schema、连接、writer queue 和生命周期。
该数据库不属于 AcTrail 主 Storage，也不属于 NoInterest Sandbox Evidence DB。

同一条告警只有在独立数据库事务提交成功后，才产生可丢弃的外发副本。
数据库写入失败时不外发未持久化告警。
数据库 queue 满、SQLite 运行期错误或 proxy 故障不得阻塞 Hand connection，也不得使 gateway、SB 或 daemon 主服务退出。

## 2. 告警判定

插件按 `(gateway_id, sb_id)` 保存有界来源状态。
每个来源状态同时记录 `guest_boot_id`。
Guest boot 变化时，内存越阈状态和 CPU baseline 全部重建。

插件产生以下告警：

- `OomKilled`：Guest 内核 `oom/mark_victim` 选中一个 OOM victim；
- `OomRisk`：可用内存从阈值以上进入阈值以下；
- `HighCpu`：相邻 CPU 累计计数形成的区间利用率从阈值以下进入阈值以上；
- `HighRead`：单个进程采样区间的读取字节数超过阈值；
- `HighWrite`：单个进程采样区间的写入字节数超过阈值。

CPU 区间利用率使用整数 basis points 表示。
首个快照、Guest boot 变化、累计计数倒退或总 tick 无增量时只重建 baseline，不产生 CPU 告警。
持续高 CPU 不重复产生告警；恢复到阈值以下后再次越过阈值才产生下一条告警。

资源类告警使用 `sampled_at_ms` 作为检测时间。
进程 I/O 告警使用 `sample_ended_ms` 作为检测时间。
OOM 告警使用内核事件的 Guest boot 单调时间换算出的检测时间。
数据库提交时间和 proxy 发送时间不得覆盖检测时间。

OOM victim observation 包含 victim PID、内核 `comm`、归因状态和可选的被观测谱系根标记。
归因状态为 `monitored`、`unmonitored` 或 `unknown`。
当前采集器只在事件发生时命中 Guest eBPF 跟踪表后输出 `monitored`；未命中时输出 `unknown`，避免把跟踪容量不足或谱系发现窗口误判为 `unmonitored`。
所有 OOM victim 都产生 `critical` 告警。
`monitored` 事件额外携带谱系根，使告警接收方可以优先处理被观测 Agent 及其后代的 OOM。

Guest 资源快照中的 `oom_kill_count` 是累计资源指标，不生成 `OomKilled` 告警。
OOM 事件 queue 的容量损失进入采集诊断。

## 3. 独立持久化

Sandbox alert store 保存结构化记录：

- 数据库生成的 alert ID 与 ingest epoch；
- gateway ID、SB ID、Guest boot ID 和 batch sequence；
- 告警类别、严重级别和检测时间；
- 可选进程标记；
- 观测值、阈值及该告警类型需要的增量字段；
- 数据库持久化时间。

数据库路径、schema version、busy timeout、writer queue、transaction batch、flush interval、retention、capacity、WAL checkpoint、synchronous mode、shutdown drain 和 reader limit 由独立的 `sandbox_alerts` 配置拥有。

启动时必须完成配置校验、目录准备、SQLite 打开、schema 校验和 writer 启动。
任一步失败时 daemon 启动失败。

运行期 producer 只对有界 queue 执行 `try_send`。
writer 线程拥有 SQLite write connection，并以有界批量事务提交。
queue 满或 writer 关闭时丢弃当前告警并更新状态计数。
已接纳告警的事务失败时记录失败状态，继续处理后续工作。

## 4. 外发标准化

Sandbox alert 在 daemon 内转换为标准 `ForwardAlert`。
它不进入依赖 `TraceId` 和主告警表的 `AlertIngress`。

外发类别和严重级别为：

| Sandbox alert | Category | Severity |
| --- | --- | --- |
| `OomKilled` | `sandbox.resource.oom_killed` | `critical` |
| `OomRisk` | `sandbox.resource.oom_risk` | `warning` |
| `HighCpu` | `sandbox.resource.high_cpu` | `warning` |
| `HighRead` | `sandbox.process.high_read` | `warning` |
| `HighWrite` | `sandbox.process.high_write` | `warning` |

Sandbox source 不包含 `trid`。
资源类 source 包含 gateway ID、SB ID 和 Guest boot ID。
进程类 source 额外包含 PID、进程启动 tick 和固定宽度二进制名的无损十六进制表示。
`monitored` OOM 的 source 使用被观测谱系根标记，victim PID、victim `comm` 和归因状态进入 `extras`。
`unknown` 与 `unmonitored` OOM 不伪造进程稳定标记。

`extras` 只保存告警业务量，不重复保存 source 身份。
外发前先检查 builtin forwarding plugin 的有效启用状态和 category filter。
未启用、不匹配、queue 满或连接断开时只丢弃外发副本，已提交的数据库记录保持不变。

## 5. 配置归属

`SandboxResourceAlertConfig` 只拥有检测配置：

- CPU 利用率阈值；
- 可用内存阈值；
- 区间读取字节阈值；
- 区间写入字节阈值；
- 来源状态容量。

CPU、可用内存、区间读取和区间写入阈值可通过 Web plugin config API 在线修改。
管理请求完成 JSON 和类型校验后，先原子替换配置文件，再发布不可变运行时配置快照。
插件 worker 每个 observation batch 只读取一次快照，同一 batch 不混用新旧阈值。
校验或持久化失败时继续使用旧配置。
配置切换不卸载插件、不更换 consumer，也不清空来源状态。
`source_state_capacity` 决定状态表结构容量，只能在插件加载前配置。

`SandboxAlertsConfig` 只拥有独立数据库及异步 writer 的运行参数。
该子配置显式启用时，daemon 在加载 Sandbox resource alert plugin 前完成数据库启动。
数据库未启用时加载该插件必须失败。

`AlertForwardingConfig` 和 builtin plugin 配置只拥有 proxy 连接及外发选择。

修改任一子配置不要求其他模块理解该配置的内部字段。
