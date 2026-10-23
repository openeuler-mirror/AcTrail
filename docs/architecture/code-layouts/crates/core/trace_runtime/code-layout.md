状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/trace_runtime/ # 管理运行中的 trace、进程归属与生命周期；持久化由应用边界协调。
└── [x] src/ # 生命周期注册表、membership 索引与采集能力协商。
    ├── [x] lib.rs # 导出 TraceRuntime 及各运行时协作模块。
    ├── [x] registry.rs # 拥有活跃 trace，并依据根进程及后代状态推进生命周期。
    │   ├── [x] struct TraceRuntime # 持有 trace ID 分配器、collector 描述和 TraceEntry 注册表。
    │   │   ├── [x] create_starting_trace # 创建 trace、根 membership、告警令牌和导出 trace 身份。
    │   │   ├── [x] activate_trace # 将已准入 trace 及其初始 membership 转为活跃状态。
    │   │   ├── [x] insert_observed_child # 登记已发生的 fork 关系，不因后续传播开关变化拒绝历史事实。
    │   │   ├── [x] track_remove_root # 关闭根进程采集及传播，依据剩余成员推进收尾。
    │   │   └── [x] complete_after_resource_barrier # 在资源控制完成后结束 draining trace，开放成员使健康状态降级。
    │   ├── [x] struct TraceEntry # 聚合 TraceRecord、固定 profile、sensor plan、membership 与 owner。
    │   └── [x] struct TraceOwnerPrincipal # 记录准入主体 UID、PID namespace 和 mount namespace。
    ├── [x] membership.rs # 按稳定 ProcessIdentity 查找和维护 trace 内进程归属。
    │   └── [x] struct MembershipIndex # 保存根与后代 membership，提供活跃采集成员判定。
    ├── [x] sensor_plan.rs # 将 profile 的必需与可选能力分配给支持它们的 collector。
    │   ├── [x] struct SensorPlan # 保存 profile 名称、collector 能力分配及未绑定的可选能力。
    │   └── [x] struct NegotiationFailure # 说明无法满足的能力与失败原因，交准入方处理。
    ├── [x] state_machine.rs # 校验 trace 状态迁移，分别维护生命周期和健康状态。
    │   └── [x] struct StateTransitionError # 返回非法迁移的原状态和目标状态。
    └── [x] commands.rs # 接收创建 trace 与移除根进程的领域请求。
        ├── [x] struct TrackTraceRequest # 携带根身份、profile 快照、标签及初始环境事实。
        └── [x] struct RootRemovalRequest # 指定待停止传播的 trace 及发生时间。
```
