# Checkpoint 保存与恢复

> 本文定义 checkpoint 在 launch、LLM 边界和恢复时的执行顺序，以及历史记录可信的条件。

Status: Proposed
Owner: checkpoint 服务
Scope: 启用 checkpoint 的 launch Trace

组件、术语与完整流程见 [Checkpoint](../../architecture/components/checkpoint.md)。LLM 边界指 daemon 从 TLS-sync 上报的明文中识别出 LLM 请求、runtime 在交付这次响应前等待放行的位置。

## 执行顺序

1. 初始 save 在 launch 子进程 exec 之前完成。daemon 在初始 checkpoint 写入记录库后才回复 `TrackAdd`，ctl 收到成功回复后才释放启动屏障；backend 连接或初始 save 失败时，launch 失败。
2. 在 LLM 边界执行保存或恢复时，runtime 收到 daemon 放行到这次请求的 `Release` 之后，才能把响应字节交给 Agent。保存失败时，daemon 同样发送 `Release`。
3. 恢复时，Trace 中仍存活的进程全部确认停止后，backend 才开始修改文件。恢复成功或执行 `CheckpointResume` 时，daemon 只继续由 checkpoint 暂停的进程。

## 历史记录

4. 一次 save 的全部 scope 都完整保存成功后，才创建 checkpoint。backend 在 `restore` 结果中报告失效的快照后，包括恢复失败时已经造成的失效，对应的 checkpoint 保留在历史中，标为不可恢复。删除 checkpoint 时，先在记录库中把它标为已删除、不可恢复，再调用 backend 的 `delete`。
5. backend 调用已经开始、但 daemon 退出而没有记录结果时，operation 记为 `unknown`，新的 daemon 不重发该调用。
6. backend 连接在 Trace 结束或用户 disconnect 时结束，此后不能再恢复。Trace 结束或 disconnect 受理后，Trace 不再接受 save 和 restore；该 Trace 此前排队的工作完成后，剩下的 checkpoint 先在记录库中标为已删除、不可恢复，再调用 backend 的 `disconnect`，它释放全部快照。

## backend 调用

7. 每个 Trace 有自己的 backend 实例，同一实例上的调用不重叠，不同 Trace 的实例同时运行。
