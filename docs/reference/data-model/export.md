# 导出 action 模型

> 本文说明语义 action 的完成边界、持久化和异步导出行为。

AcTrail 异步导出已经完成的语义事实。导出失败不能阻塞观测、治理或主存储写入。

## Action 类型

| Action | 完成边界 | 持久化与导出行为 |
| --- | --- | --- |
| `process.exec` | 一次完成的 process image replacement | 持久化一个语义事实，并提交一次在线导出 |
| `command.invocation` | 一次完成的命令调用 | 持久化一个语义事实，并提交一次在线导出 |
| `llm.request` | 一次完成且已保留的 LLM request | 持久化一个语义事实，并提交一次在线导出 |
| `agent.identity` | 首次确认 Agent identity | 只导出一次；后续 request 不刷新它 |
| `process.exit` | Process 终止 | 提交 terminal 在线导出，不重放之前的 action |
| `agent.exit` | Agent 终止 | 提交 terminal 在线导出，不重放之前的 action |
| `file.read` | File lifecycle close 或其他显式 terminal boundary | 每个 read direction 产生一个有界 aggregate |
| `file.write` | File lifecycle close 或其他显式 terminal boundary | 每个 write direction 产生一个有界 aggregate |

持久化与异步导出是相互独立的 best-effort 结果，即系统会尝试完成两者，但两者之间不保证原子性或顺序。Queue 满时丢弃新的 export record，不替换较早的 queued record；失败必须产生结构化 diagnostic 和 counter。

## File I/O 聚合

同一 open/close lifecycle 内，read 与 write direction 分别聚合。有界 summary 包含 open、first I/O、last I/O、close、total bytes、operation count 和 error count；任一 I/O 失败都会使 terminal status 成为 error。

Close 是正常边界。Process exit、trace finalization、graceful shutdown 和 file descriptor replacement 可以把现有 state 作为 partial 结束，但不能伪造 close evidence。Capacity eviction 不是 lifecycle boundary，不能据此生成 partial action。

规范性行为见 [语义 action delivery](../../specifications/export/action-delivery.md) 和 [File I/O terminal action](../../specifications/export/file-io-terminal-actions.md)。
