# Checkpoint Backend 接口

> 本文定义 checkpoint backend 动态库要实现的 ABI 接口。

backend 负责受保护目录的文件状态保存与恢复，并管理由此产生的快照或日志。它是实现 `crates/contracts/checkpoint` 契约的动态库，由 daemon 在进程内加载，契约用 `abi_stable` 保证跨库的二进制兼容。仓库中的 `integrations/checkpoint/filesystem/btrfs` 和 `agentns` 是按本文实现的 backend。

AcTrail 负责保存时机、LLM 响应等待、进程暂停、按 Trace 调度和历史记录。组件设计见 [Checkpoint](../architecture/components/checkpoint.md)。

## 动态库入口

动态库用 `#[export_root_module]` 导出 `CheckpointModule`。`create` 接收配置、创建一个未连接的实例；`kind` 声明类型：`Filesystem` 保护 launch 配置点名的目录，没有被点名时不参与这次 Trace；`Context` 没有 scope，每个 Trace 都参与。配置是 `[[checkpoint.backends]]` 的 `config` 文件的完整内容，由 backend 自己定义格式并校验，daemon 不解析。

```rust
#[export_root_module]
fn module() -> CheckpointModuleRef {
    CheckpointModule { create, kind: BackendKind::Filesystem }.leak_into_prefix()
}
```

每个 Trace 用一个新实例，调用 `create` 的次数不固定。

## 方法

| 方法 | 调用时机 | 结果 |
| --- | --- | --- |
| `connect(context)` | Trace 建立时，在 launch 启动屏障内、Agent 执行之前 | 把实例绑定到这个 Trace，之后的调用都针对它 |
| `snapshot()` | 初始保存、需要保存的自动保存和手动保存 | 返回 `SnapshotId`；失败时自己清理已产生的内容 |
| `restore(snapshot_id)` | 恢复时；调用前 Trace 中仍存活的进程已全部暂停 | 返回这次恢复导致失效的 `SnapshotId` 列表，失败时由 `RestoreFailure` 带回已经造成的失效；目标本身保持可用 |
| `delete(snapshot_id)` | 用户删除 checkpoint | 释放这个快照独占的内容，不影响其他快照 |
| `disconnect()` | Trace 结束、用户 disconnect、daemon 正常停止或 launch 失败 | 结束连接并释放仍持有的全部快照 |

`connect` 失败时 launch 失败。`disconnect` 成功之后不再调用这个实例；失败之后只会再次调用 `disconnect`。

`SnapshotContext` 携带 Trace 编号、Trace 根进程（`host_pid`、`start_time_ticks`、`pid_namespace`、`mount_namespace`、`cwd`）和这个实例负责的 scope（`id`、`root`、`excludes`）。daemon 已完成 scope 嵌套的划分，backend 按收到的 `root` 和 `excludes` 处理；无法落实时 `connect` 返回错误。`Context` 类 backend 收到的 `scopes` 为空。需要追踪 Agent 修改的 backend 在 `connect` 中关联根进程，此时根进程停在启动屏障内，尚未执行 Agent 程序。

`SnapshotId` 由 backend 定义，AcTrail 只保存和回传，不解析。它只在发放它的连接内有意义，不需要跨进程或跨连接有效。

## 调用顺序

同一实例上的调用不重叠。不同 Trace 的实例同时运行，访问同一底层状态时由 backend 自行协调。一个 Trace 的多个 backend 由 AcTrail 的组合 backend 按配置顺序依次调用，某一个失败后不再调用后面的。

调用是同步的，在 daemon 的 worker 线程上执行，没有统一的超时。

## 失败语义

失败以 `BackendError` 返回，`restore` 的失败把它放在 `RestoreFailure.error` 里，`kind` 为 `InvalidConfiguration`、`InvalidSnapshot`、`Unavailable` 或 `OperationFailed`，不表示重试是否安全。

| 情况 | AcTrail 行为 |
| --- | --- |
| `connect` 失败 | launch 失败 |
| `snapshot` 失败 | 不创建 checkpoint，失败原因记在 operation 上；初始保存使 launch 失败，自动保存放行响应、Agent 继续运行 |
| `restore` 失败 | 本次恢复失败，保持暂停；`RestoreFailure.invalidated` 列出的快照照常记入历史。恢复到已失效的快照返回 `InvalidSnapshot` |
| `delete` 失败 | checkpoint 仍为已删除，失败原因记在 checkpoint 上，用户再次删除时重试 |
| `disconnect` 失败 | Trace 仍记为已 disconnect，失败原因记在 Trace 和这次删除的 checkpoint 上。实例保留，用户再次 disconnect 或 Trace 结束时重试 |

错误必须作为返回值传回，不能让 panic 越过 ABI 边界。backend 崩溃时 daemon 随之退出。

## 实现注意事项

- 方法返回前完成对受保护文件的全部修改。
- AgentNS/RNS 实现须选择进程树捕获，并在恢复时保留 coordinator 已暂停的进程；所用 SDK 和运行中模块必须支持这两个模式。
- `snapshot` 成功时，这个快照已可用于恢复。
- 恢复会覆盖目标之后尚未保存的修改，这是正常输入。
- 为完成恢复而维护的内部锚点或日志，不作为快照返回。
- `delete` 只释放快照独占的数据；仍被其他快照依赖的数据，等依赖消失后再释放。例如 AgentNS 的日志从第一个 checkpoint 起连续保存，删除其中一部分不释放空间，`disconnect` 时才释放整个区域。
- 已不存在或已失效的快照，`delete` 视为删除成功，重复删除是安全的。
- 调用 `disconnect` 时 Trace 的进程可能仍在运行，例如用户在 Trace 运行中执行 disconnect。
- `disconnect` 失败时尽量释放能释放的部分，再次调用时只处理剩下的。
- 连接状态放在实例内存中即可。daemon 异常退出后实例不再存在，AcTrail 不要求恢复连接；这时留下的快照需要运维手动清理。
