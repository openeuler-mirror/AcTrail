# 导出约束

- [ ] 导出必须异步、在线，不得阻塞上游执行。
- [ ] 一次完成的动作只导出一次，不得通过重复快照表达同一事实。
  - [ ] 每个 `process.exec` 必须作为独立动作。
    - [ ] 只能由进程镜像替换完成的观测触发，与该程序之后的退出码无关。
    - [ ] seccomp exec 观测只表示执行意图，仅用于补充参数，不得独立生成动作。
    - [ ] 同一次 exec 的 seccomp 与 eBPF 观测必须合并；缺少 seccomp 时仍由 eBPF 生成动作。
    - [ ] 同一进程顺序执行的不同 exec 必须存储为不同动作。
    - [ ] 进程身份由 fork、launch 或 attach 建立；exec 只关联进程身份，不得作为进程身份。
  - [ ] 每个 `llm.request` 独立导出一次。
  - [ ] agent identity 仅在首次确认时生成一个动作。
    - [ ] 后续 request 不得重复生成或刷新该动作。
  - [ ] process 或 agent 结束时只生成对应的结束动作。
    - [ ] 不得重发已经完成的 exec、request 或 identity 动作。

## 持久化边界

- `process.exec`、`command.invocation`、`llm.request` 和 `agent.identity` 是可查询的语义事实，写入 semantic action 存储并在线导出。
- `process.exit` 和 `agent.exit` 只用于在线导出。进程终态已由 process record 和原始 exit event 持久化，禁止为导出动作重复扩大语义存储。

## Exec intent 关联与容量

- `process_seccomp.pending_max_entries` 同时限制等待物化的 seccomp process observation
  和等待 eBPF completion 的 exec intent 总数；达到容量时逐出最早 intent，并输出聚合
  warning。completion 仍生成语义完整但参数可能较少的 `process.exec`。
- intent 只能与同一逻辑进程队首、可唯一确认的 completion 合并。可执行路径不一致、
  连续同路径 intent 产生歧义，或候选顺序无法证明时，必须降级为 completion-only，
  不得跨过更早 intent 猜测关联。
- completion 没有可执行路径时，只允许合并该进程唯一的 pending intent；多个候选时
  同样降级，避免把失败尝试的参数附到后续成功 exec。

## One-shot action 的 link 修订

- fork 父身份后续出现冲突时，只写入无效化 link，不得为传播 link 变化而重发已完成的
  `command.invocation`、`process.exec` 或 `agent.identity`。
- 当前 OTEL live span 协议没有独立 link-update 记录，link-only invalidation 会进入
  持久化语义关系，但不会改写已经在线导出的 one-shot span。需要最终关系真值的消费方
  应使用 post-trace 查询中的 `valid` link；在线协议若要表达 link 修订，必须另行设计
  独立记录类型，不能恢复 action 快照刷新。
