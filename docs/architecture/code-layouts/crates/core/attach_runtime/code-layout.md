状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/attach_runtime/ # 协调已有进程的采集绑定、树快照与身份合并，记录 bootstrap 覆盖情况。
└── [x] src/ # 接入协调器及快照合并步骤。
    ├── [x] lib.rs # 导出接入请求、结果和 BootstrapCoordinator。
    ├── [x] bootstrap.rs # 借用 collector、身份读取器和快照源，完成已有进程接入。
    │   ├── [x] struct BootstrapCoordinator # 使用请求中的 sensor plan，编排身份读取、采集绑定与快照交接。
    │   │   └── [x] attach_existing # 消费接入请求并返回 bootstrap 结果或带阶段的失败。
    │   ├── [x] struct AttachRequest # 指定根进程、profile 和 trace 的初始属性。
    │   └── [x] struct BootstrapResult # 返回 trace、根身份、接入时刻和 bootstrap 覆盖不完整标记。
    ├── [x] coverage_guard.rs # 将采集与快照时刻转换为 bootstrap 覆盖窗口。
    │   ├── [x] struct BootstrapTimings # 保存绑定、快照开始及结束等关键时间。
    │   └── [x] struct CoverageWindow # 表达快照和实时采集衔接的覆盖范围。
    ├── [x] identity_merge.rs # 对比快照与实时事件中的进程观察集合。
    │   └── [x] struct IdentityMergeSummary # 区分快照独有、实时事件独有及两者共有的进程观察。
    └── [x] snapshot_merge.rs # 将快照进程关系转换为 trace membership。
        └── [x] struct SnapshotMergeResult # 汇总快照合并后的成员与覆盖结果。
```
