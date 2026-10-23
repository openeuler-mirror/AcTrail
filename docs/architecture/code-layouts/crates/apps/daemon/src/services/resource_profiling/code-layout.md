状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/daemon/src/services/resource_profiling/ # 资源组件的应用适配；提供 trace/进程事实、生命周期通知与控制状态持久化。
├── [x] cgroup_v1/ # 适配 cgroup v1 绑定恢复、关闭事件与资源控制状态持久化。
│   ├── [x] event_sink.rs # 持久化 v1 绑定关闭并确认采集会话结束。
│   ├── [x] mod.rs # 声明模块。
│   └── [x] persistence.rs # 适配 v1 绑定恢复、失效、关闭和 trace 健康状态。
│       └── [x] struct V1StorageError # 将存储错误映射为资源采集错误。
├── [x] external_scope_storage.rs # external 恢复事务、连续失败计数与失效状态。
│   └── [x] struct ExternalScopeStorage # external 恢复事务、连续失败计数与失效状态。
├── [x] managed_scope_storage.rs # 保存 managed 范围恢复、关闭与清理状态。
│   └── [x] struct ManagedScopeStorage # 管理自建范围控制记录和 trace 生命周期持久化。
├── [x] mod.rs # 声明模块并 re-export。
├── [x] resource_profiling_adapter.rs # 在文件权限拦截启动前构造资源组件，注入应用接口并转交调度与生命周期通知。
│   └── [x] struct ResourceProfilingAdapter # 持有 ResourceProfilingService，适配 daemon 的接入、采样驱动与结束通知。
├── [x] scope_persistence_adapter.rs # 将组合业务端口委托给所属存储适配器。
│   └── [x] struct ScopePersistenceAdapter # 将组合业务端口委托给所属存储适配器。
├── [x] workload_event_sink.rs # 适配 trace 健康状态、控制完成请求与生命周期持久化。
└── [x] workload_target_adapter.rs # 读取权威 trace/进程注册表，保留 observer 时间命名空间换算。
    └── [x] struct WorkloadTargetAdapter # 读取权威 trace/进程注册表，保留 observer 时间命名空间换算。
```
