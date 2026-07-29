# File I/O Semantic Action 终态导出设计

## 状态

目标设计。本文只保留已经确认的关键产品约束；数据结构、命名和具体算法由实现阶段
决定。

## 1. 背景

聚合型 file I/O action 会随每次 I/O 更新累计字节数、次数和 evidence。重复持久化
并导出完整快照，会放大数据库写入和 OTEL/JSONL 输出；不断追加全部 evidence 还会
使处理成本从 O(n) 放大到 O(n²)。

目标是在保持异步、在线导出的同时，每个聚合 action 只形成一个终态版本。

## 2. Action 生命周期

- One-shot action 生成时已经是终态，立即进入异步导出。
- 聚合型 action 在结束边界前只维护有界内部状态，不形成正式中间态
  `SemanticAction`。
- 结束边界到达后，正式 action 只构造并持久化一次，也只向 exporter 提交终态。
- 同一 open/close 生命周期内，read 与 write 分别聚合为 `file.read` 和
  `file.write`。
- 正常 close 是 file I/O 的结束边界。缺失 close 时，process exit、trace
  finalize、graceful shutdown 或 FD replacement 将已有聚合收口为 Partial，且不
  伪造 close evidence。
- 另一个 action 的开始只有通过显式登记的转换规则，并匹配前一个 action 声明的
  聚合 scope，才能成为结束边界。FD action 匹配同一 handle；process-level burst
  匹配同一 trace 与 process。
- Open 后没有发生 I/O 时，不生成 read/write action。

`file.bulk_read` 和 `fs.enumerate` 同样只输出终态：

- burst summary 达到阈值时只在内存中激活，在显式结束边界或 trace finalize 输出；

`file.tty_io` 不进入 observation export。TTY 的识别与内部生命周期属于上游投影，
导出链路不得通过路径、设备身份或 handle 状态重新识别，只在统一 action 导出入口
按 kind 排除。

见 [ADR-0001](adr/0001-terminal-action-lifecycle.zh.md)。

## 3. 默认摘要

聚合型 file I/O action 默认保留：

- open；
- first I/O；
- last I/O；
- close；
- 累计字节数；
- 累计 I/O 次数；
- 累计错误次数。

默认摘要必须为 O(1)。只有一次 I/O 时，该事件只引用一次；`io_count=1` 表明它同时
是 first 和 last。任一次 I/O 失败都会使终态 status 为 Error，后续成功不能覆盖。

默认 relationships 只保留 action-level parent/lineage，不累计逐 I/O
relationships。Raw event 是否保存及保存多久由独立 retention 策略决定。

见 [ADR-0002](adr/0002-bounded-file-io-summary.zh.md)。

## 4. 单一事实表示

同一批底层事实只能由一种 semantic action 表示：

- summary action 接管后，不再输出对应 detailed action；
- write/writev 已归入聚合 `file.write` 时，不再逐次输出 `file.modify`；
- 无法关联 open state 的 I/O，只有在事件自身语义完整时才形成 one-shot；信息不足
  时不伪造 action；
- 无法关联 open、但语义完整的 write/writev 只生成 one-shot `file.write`，不再为
  同一事实生成 `file.modify`。

见 [ADR-0003](adr/0003-exclusive-fact-representation.zh.md)。

## 5. 在线异步交付

持久化与异步导出独立尽力而为，不要求先持久化再导出，也不要求二者原子一致。

Export queue 满时丢弃当前新记录，不替换已排队记录。持久化失败、queue 满、编码
失败和写出失败必须产生结构化诊断与计数，不允许静默丢失。

见 [ADR-0004](adr/0004-independent-best-effort-delivery.zh.md)。

## 6. 实现约束

- 每个聚合状态及默认输出均为 O(1)。
- 活跃状态总量与异步队列必须有界。
- 容量淘汰不是生命周期边界，不能伪造 Partial 终态。
- 被淘汰 handle 在真实边界到达前不得降级为逐 I/O one-shot。
- 正式 action 的存储与发布入口只接收终态；不要求给 `SemanticAction` 增加
  lifecycle 字段。

实现可以自行选择数据结构、容量默认值、evidence role 名称和诊断内部载体，但不得
改变本文定义的生命周期、聚合 scope、事实归属和输出语义。
