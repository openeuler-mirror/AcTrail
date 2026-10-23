状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/linux_cgroup/ # 操作 Linux cgroup v2 层级、目录及计数文件，向资源组件提供内核访问能力。
└── [x] src/ # 层级预检、受管目录生命周期和计数读取实现。
    └── [x] lib.rs # 定义 cgroup v2 的目录操作与计数模型。
        ├── [x] struct ManagedCgroupV2 # 持有受管根路径，创建和回收 trace 对应的内核目录。
        │   ├── [x] prepare_hierarchy # 检查委派并准备 controller 层级。
        │   ├── [x] create_trace_scope # 创建 aggregate 与 workload 目录。
        │   ├── [x] move_pid # 将目标进程写入 workload 的成员列表。
        │   ├── [x] scan_managed_scopes # 在数量预算内扫描可恢复的 trace 范围。
        │   └── [x] remove_empty_trace_scope # 删除已无成员的 trace 范围。
        ├── [x] struct CgroupV2CounterReader # 保留绑定目录句柄和 memory.peak 读取句柄，持续读取同一目录对象。
        │   ├── [x] from_open_directory # 接收已核验目录句柄，避免通过显示路径重新绑定对象。
        │   └── [x] read_counters # 读取 CPU、内存、I/O 及 populated 状态。
        ├── [x] struct UnifiedHierarchy # 保存 cgroup2 挂载根和当前进程所属层级路径。
        ├── [x] struct CgroupPreflightReport # 返回委派所有权、controller、daemon 位置和探测结果。
        ├── [x] struct TraceScopePaths # 描述一个 trace 的相对路径、aggregate 与 workload 路径。
        ├── [x] struct ManagedScopeDirectory # 关联扫描出的 trace ID、nonce 与范围路径。
        ├── [x] struct CgroupCounters # 汇总资源计数及各可选计数的可用情况。
        └── [x] enum CgroupError # 区分内核访问、目录结构与计数解析错误。
```
