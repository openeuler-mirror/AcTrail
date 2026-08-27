# 语义动作交付

> 本文定义语义事实的存储、导出、信息补充与禁止重放条件。

Status: Accepted
Owner: 语义存储与导出运行时
Scope: action materialization、持久化与在线导出

semantic action 是 AcTrail 对有意义的 agent 或进程事实提供的可查询表示。导出必须异步，不能阻塞上游执行。一个完成事实只导出一次，禁止用变化中的 snapshot 重复表示同一事实。

- 每次完成的进程镜像替换生成一个 `process.exec`；seccomp exec observation 只是补充 completion 的 intent，不能独立生成 action。
- 每个 `llm.request` 只导出一次。
- agent identity 只在首次确认时导出，后续 request 不刷新。
- process 与 agent 结束只生成各自终态 export record，不重放早先 action。

`process.exec`、`command.invocation`、`llm.request` 和 `agent.identity` 是可查询语义事实：写入 semantic action store 并提交在线导出。存储与导出无顺序保证，也不具备原子性。`process.exit` 与 `agent.exit` 只在线导出，因为 process record 和 raw exit observation 已持久化终态。

pending exec intent 总量受 `process_seccomp.pending_max_entries` 约束。只有同一 logical process 的队首 completion 且关联可唯一证明时才能合并。path 不同、同 path 重复导致歧义或顺序不可证明时，必须生成 completion-only。completion 缺少 path 时只能使用唯一 pending candidate。

fork parent identity 后续失效时只新增 invalidation link，禁止重放 one-shot action。需要最终 link 真值的 consumer 使用持久化 relation；未来若要在线表达 link update，必须另行设计独立 protocol record。
