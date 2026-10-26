状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/linux_platform/ # 提供 Linux 能力预检和 seccomp 系统调用映射，供控制与采集适配器使用。
└── [x] src/ # 平台检查及架构相关系统调用编号转换。
    ├── [x] lib.rs # 导出平台模块，并将 linux_cgroup 重导出为 cgroup_v2。
    ├── [x] capability_probe.rs # 检查 Unix socket 与 no_new_privs 能力。
    │   └── [x] struct CapabilityStatus # 返回能力名称、是否可用和检查详情。
    ├── [x] file_seccomp.rs # 将配置中的文件系统调用映射为内核编号及路径参数位置。
    │   └── [x] enum KernelFileSeccompSyscall # 标识受支持的内核文件调用，解析 seccomp_data 中的调用类型。
    └── [x] process_seccomp.rs # 将进程控制配置展开为架构支持的内核系统调用。
        └── [x] enum KernelProcessSyscall # 转换进程调用编号和配置名称，统一调用匹配语义。
```
