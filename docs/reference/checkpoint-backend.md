# Checkpoint Backend 接口

> 本文定义 checkpoint backend 动态库要实现的 ABI 接口。

backend 负责文件系统或 Agent 上下文的保存与恢复，并管理由此产生的快照或日志。它是实现 `crates/contracts/checkpoint` 契约的动态库，由 daemon 在进程内加载，契约用 `abi_stable` 保证跨库的二进制兼容。仓库提供 Btrfs、AgentNS 文件系统 backend 和 Xiaoo 上下文 backend。

AcTrail 负责保存时机、LLM 响应等待、进程暂停、按 Trace 调度和历史记录。组件设计见 [Checkpoint](../architecture/components/checkpoint.md)。

## 动态库入口

动态库用 `#[export_root_module]` 导出 `CheckpointModule`：

```rust
#[export_root_module]
fn module() -> CheckpointModuleRef {
    CheckpointModule { create, kind: BackendKind::Filesystem }.leak_into_prefix()
}
```

daemon 启动时按 `[[checkpoint.backends]]` 的 `name` 和 `library` 加载实现，并读取 `config` 指定的运行配置文件，将原文保存在内存。运行配置在这次 daemon 生命周期内固定。

启动时检查动态库和运行配置文件能否读取。配置格式由 backend 在创建实例时校验。

launch checkpoint 配置的 `[[backends]]` 选择本次 Trace 使用的实现，并通过可选的 `config` 指定 Trace 私有配置路径。ctl 读取 checkpoint 配置后透传。daemon 调用 `create(BackendConfig)`，其中 `runtime_json: RString` 是运行配置原文，`trace_config_path: ROption<RString>` 是本次 Trace 的私有配置路径。

backend 在 `create` 时解析运行配置，并按自身需要读取、解析 Trace 私有文件，然后创建未连接的实例。两份 JSON 的结构、必需字段和校验均属于 backend。

`kind` 声明实例的调用时机：`Filesystem` 保护分配给它的 scope，`Context` 保护上下文。

每个 Trace 为其选中的各 backend 创建一个新实例。不同 launch 可以选择同一个 backend 名称，共用运行配置并提供各自的 Trace 配置。backend 在 `connect` 时绑定 Trace，在 `disconnect` 时释放这个 Trace 的资源。

Btrfs 和 AgentNS 的目标来自 `SnapshotContext`，launch 可省略它们的 `config`。Xiaoo 用 Trace 配置提供 `endpoint`。

## 方法

| 方法 | 调用时机 | 结果 |
| --- | --- | --- |
| `connect(context)` | 文件系统 backend 在 launch 启动屏障内连接。上下文 backend 在首个 LLM 请求边界或首次手动保存时连接 | 把实例绑定到这个 Trace，之后的调用都针对它 |
| `snapshot()` | 只有文件系统 backend 时的初始保存，以及后续自动保存和手动保存。首次在 LLM 边界连接上下文时跳过该次保存 | 返回 `SnapshotId`。失败时自己清理已产生的内容 |
| `restore(snapshot_id)` | 文件系统 backend 在 Trace 存活进程全部确认暂停后调用。上下文 backend 在文件恢复成功、进程继续后调用，此时仍保持 LLM 响应等待 | 返回这次恢复导致失效的 `SnapshotId` 列表。失败时由 `RestoreFailure` 带回已经造成的失效，目标本身保持可用 |
| `delete(snapshot_id)` | 用户删除 checkpoint | 释放这个快照独占的内容，不影响其他快照 |
| `disconnect()` | Trace 结束、用户 disconnect、daemon 正常停止或 launch 失败 | 结束连接并释放仍持有的全部快照 |

启动阶段的文件系统 `connect` 失败时 launch 失败。运行中的上下文连接失败通过当前边界或手动保存返回，Agent 继续运行。

手动保存连接上下文后立即调用 `snapshot`，尚无完整上下文时由 backend 返回失败。

`disconnect` 成功后结束实例调用。失败后只允许再次调用 `disconnect` 重试释放。

`SnapshotContext` 携带 Trace 编号、Trace 根进程（`host_pid`、`start_time_ticks`、`pid_namespace`、`mount_namespace`、`cwd`）和这个实例负责的 scope（`id`、`root`、`excludes`）。`Context` 类 backend 收到的 `scopes` 为空。

daemon 完成 scope 嵌套划分后，backend 按收到的 `root` 和 `excludes` 处理。无法落实时，`connect` 返回错误。

文件系统 backend 连接时，根进程停在启动屏障内，可以在 Agent 执行前建立修改追踪。上下文 backend 延迟连接，接收 launch 时记录的同一份根进程信息。

`SnapshotId` 由 backend 定义和解释，AcTrail 保存并原样回传。ID 的有效范围是发放它的连接。

## 调用顺序

同一实例上的调用不重叠。不同 Trace 的实例同时运行，访问同一底层状态时由 backend 自行协调。

组合 backend 按 launch 的 `backends` 顺序保存。恢复先调用文件系统 backend，再调用上下文 backend，同类内保持该顺序。某一个恢复调用失败后，后面的恢复调用停止。

调用是同步的，在 daemon 的 worker 线程上执行，没有统一的超时。

Xiaoo 的成功响应表示请求已受理，排队的保存和恢复在 Agent 完整循环边界处理。daemon 收到上下文恢复成功后放行 LLM 响应。详情见 [Xiaoo 集成](../../integrations/checkpoint/context/xiaoo/README.md)。

## 失败语义

失败以 `BackendError` 返回，`restore` 的失败把它放在 `RestoreFailure.error` 里，`kind` 为 `InvalidConfiguration`、`InvalidSnapshot`、`Unavailable` 或 `OperationFailed`，不表示重试是否安全。

| 情况 | AcTrail 行为 |
| --- | --- |
| `connect` 失败 | 文件系统连接失败使 launch 失败。运行中上下文连接失败返回错误并放行响应 |
| `snapshot` 失败 | 失败原因记在 operation 上，本次不创建 checkpoint。初始保存失败使 launch 失败。自动保存失败时放行响应，Agent 继续运行 |
| `restore` 失败 | 本次恢复失败，文件系统失败时保持暂停，上下文失败时再次暂停进程。`RestoreFailure.invalidated` 列出的快照照常记入历史。恢复到已失效的快照返回 `InvalidSnapshot` |
| `delete` 失败 | checkpoint 仍为已删除，失败原因记在 checkpoint 上，用户再次删除时重试 |
| `disconnect` 失败 | Trace 仍记为已 disconnect，失败原因记在 Trace 和这次删除的 checkpoint 上。实例保留，用户再次 disconnect 或 Trace 结束时重试 |

错误必须作为返回值传回，不能让 panic 越过 ABI 边界。backend 崩溃时 daemon 随之退出。

## 实现注意事项

- 文件系统 backend 的方法返回前完成对受保护文件的全部修改。
- `snapshot` 成功时，返回的 ID 可以按 backend 的恢复模式提交。Xiaoo 的 `promise_mode` 接受已排队的快照，`ensure_safe_mode` 要求快照已保存。
- 恢复会覆盖目标之后尚未保存的修改，这是正常输入。
- 为完成恢复而维护的内部锚点或日志，不作为快照返回。
- `delete` 按引用关系释放数据：快照独占的数据立即释放，共享数据在其他快照的依赖消失后释放。例如 AgentNS 的日志从第一个 checkpoint 起连续保存，删除其中一部分不释放空间，`disconnect` 时才释放整个区域。
- 已不存在或已失效的快照，`delete` 视为删除成功，重复删除是安全的。
- 调用 `disconnect` 时 Trace 的进程可能仍在运行，例如用户在 Trace 运行中执行 disconnect。
- `disconnect` 失败时尽量释放能释放的部分，再次调用时只处理剩下的。
- 连接状态保存在实例内存中，有效期为本次 daemon 运行。daemon 异常退出后实例消失，遗留快照由运维手动清理。

## 实现说明

### Btrfs

Btrfs backend 保存只读子卷快照，恢复时将文件复制回原子卷。

连接期间持有受保护子卷的目录句柄。每次保存和恢复先确认原路径仍对应这个子卷，随后通过句柄定位文件。根目录或上级目录被移动、删除或替换时，该目录的保存和恢复返回错误。

保存和恢复使用 `snapshot_root/locks/<子卷UUID>.lock` 串行协调。操作同一子卷的 backend 实例须共用 `snapshot_root`。锁文件在 disconnect 后保留，使仍持有或等待锁的其他实例继续使用同一个锁文件。

| 条件 | backend 行为 |
| --- | --- |
| 保护范围 | scope 根须为 Btrfs 子卷根，并位于 `allowed_workspace_roots` 内。scope 须覆盖整个子卷 |
| 嵌套子卷 | connect 时拒绝已有嵌套子卷。运行中新建的子卷不在快照内，恢复保留其内容 |
| 嵌套挂载 | 恢复先按挂载表跳过挂载点，再读取其他条目的元数据。跨设备挂载、同设备 bind mount 和用户 FUSE 挂载的内容均保留 |
| 快照目录 | `snapshot_root` 与受保护子卷位于同一个 Btrfs 文件系统，且与 `allowed_workspace_roots` 互不包含。初始化将其 `snapshots` 目录设为 0700，包括已有目录 |
| 路径视图 | Agent 和 backend 须在同一个 mount namespace |

`copy_batch_max_bytes` 限制每次 `cp` 的源路径参数预算，默认 65536 字节。预算按路径字节、结束符和指针大小估算。超过预算的单个路径仍单独复制。

### AgentNS / RNS

AgentNS backend 使用 RNS 的内核模块与 C SDK。`rnsctl start` 启动 `rns.ko`，运行配置的 `library_path` 指向配套安装的 `librns.so`。模块、BPF、SDK 和 backend 须配套，提供 `rns_capabilities`、进程树捕获和保留暂停进程恢复能力。缺少能力时实例初始化失败。

实例创建会话时选择进程树捕获。Agent 的线程和后代跟随当前保存窗口，Unix socket 连接不把 daemon 等外部服务纳入会话。

backend 接受一个以 `/` 为根的 scope，合并该 scope 的排除路径、运行配置的统一附加排除和 `area_root`。与 Btrfs scope 组合时，scope 规划把 Btrfs 保护目录加入 AgentNS 的排除范围。

恢复前，coordinator 暂停全部受控进程。RNS 检查整个线程组已暂停并排空在途 I/O，恢复文件后保留这些进程的暂停状态，随后由 coordinator 继续进程。

已受理的恢复中断时，RNS 保留请求并冻结区域，保存和受管写入可能返回 `EAGAIN`。

| 后续操作 | 行为 |
| --- | --- |
| 再次恢复 | 先使用相同请求和恢复策略完成未完成的请求 |
| `CheckpointResume` | 继续进程并放行响应等待，RNS 区域仍保持冻结 |
| `disconnect` | 放弃区域并释放其保护资源 |
