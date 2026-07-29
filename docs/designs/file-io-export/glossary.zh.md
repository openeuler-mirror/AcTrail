# File I/O 终态导出术语表

| 术语 | 含义 |
| --- | --- |
| one-shot action | 生成时已经终态、之后不再更新的 action。 |
| 聚合型 action | 在结束边界前累计多个事件，只在终态形成正式 semantic action。 |
| 终态 | Action 此后不再更新；可以是 Complete，也可以是 Partial。 |
| Partial | 已经终态，但缺少正常 close 等完整生命周期信息；不得伪造 evidence。 |
| 默认摘要 | Open、first/last I/O、close evidence 与累计 bytes/count/error_count 的有界表示。 |
| 单一事实表示 | 同一批底层事实只由 summary 或 detailed 等一种 action 表示。 |
| action-level relationship | 将 action 放回调用链所需的 parent/lineage，不包含逐 I/O 关系。 |
| 独立尽力而为 | 持久化与导出没有顺序和原子一致性保证，允许任一侧单独失败。 |
| 聚合 scope | 用于匹配同一聚合生命周期的身份；FD action 使用 handle，process-level burst 使用 trace+process。 |
