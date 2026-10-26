状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/process_identity/ # 将不同 PID 坐标收敛到稳定身份；身份解析不拥有 trace membership。
└── [x] src/ # 身份值、来源端口及索引维护。
    ├── [x] lib.rs # 导出身份模型、读取端口和管理器。
    ├── [x] contract.rs # 区分逻辑身份、observer 坐标、namespace alias 和内核坐标。
    │   ├── [x] struct ProcessIdentity # AcTrail 分配的稳定数值 ID，供事件、membership 和存储关联。
    │   ├── [x] struct HostProcessCoordinates # daemon 可见 PID、开始时间与可选线程信息。
    │   ├── [x] struct NamespaceProcessCoordinates # namespace 身份、其中的 PID 与进程代际。
    │   ├── [x] struct KernelProcessCoordinates # 内核 PID 与开始时间，用于内核侧资源及 FD 关联。
    │   ├── [x] struct ProcessObservation # 一次身份读取中的 observer 坐标和可选 namespace 坐标。
    │   ├── [x] struct ProcessRecord # 同一稳定身份的坐标、alias 集合与解析状态。
    │   ├── [x] trait ProcessIdentityReader # 根据可见 PID 返回身份观测或明确的读取错误。
    │   └── [x] struct ProcessSuppressedFd # 将内部 FD 用途绑定到内核进程代际，避免误采内部通道。
    └── [x] manager.rs # 维护身份记录及精确坐标索引，处理 alias 补充和活跃 PID 复用。
        ├── [x] struct ProcessIdentityManager # 拥有身份分配区间、记录、代际索引和活跃 PID 索引。
        │   ├── [x] resolve_or_create # 查找唯一候选，分配或补充身份，并返回被替换的活跃映射。
        │   ├── [x] install_reserved_block # 在当前区间耗尽后接入连续预留的 ID 区间。
        │   └── [x] mark_exited # 移除活跃坐标映射并保留可供历史关联的身份记录。
        ├── [x] struct ProcessResolution # 返回身份、是否创建或补充及被置换的活跃身份。
        └── [x] enum ProcessIdentityError # 区分 ID 耗尽、多个候选和 observer 坐标冲突。
```
