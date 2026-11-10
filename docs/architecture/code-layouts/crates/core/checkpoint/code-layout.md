状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/checkpoint/ # 协调文件状态的保存与恢复；拥有 scope 规划、每个 Trace 的执行顺序和 backend 的组装。
├── [x] Cargo.toml # 依赖 contracts/checkpoint 与动态库加载能力。
└── [x] src/ # 记录模型、协调流程、backend 组装与外部能力端口。
    ├── [x] lib.rs # 导出协调器、factory、配置项、记录模型与端口；backends 模块保持私有。
    ├── [x] config.rs # 模块配置：描述组合结构、动态库路径及对应集成配置。
    │   └── [x] struct BackendEntry # 一个 backend 的名字、动态库路径和配置文件路径。
    ├── [x] factory.rs # 按配置加载动态库；为每个 Trace 创建参与的 backend 的新实例，组装后返回统一 CheckpointBackend；文件系统类在 scope 点名时参与，上下文类始终参与。
    │   └── [x] struct CheckpointFactory # 按配置顺序持有已加载的集成。
    ├── [x] backends/ # 仅供 factory 组装的内部实现。
    │   ├── [x] mod.rs # 声明内部后端。
    │   ├── [x] composite.rs # 实现 RestorableCheckpointBackend：把 scope 分给各子实例，依次调用，把子快照合成一个快照；保存失败时删除已得到的子快照，恢复时把子实例报告的失效换算成组合快照。
    │   └── [x] dynamic.rs # 按配置加载 .so、校验 ABI 并取得统一后端对象。
    ├── [x] model.rs # Trace、checkpoint 与 operation 的记录模型。
    │   ├── [x] struct TraceRecord # 保存 Trace 的所有者、根进程、scope 配置、结束时间和 disconnect 结果。
    │   ├── [x] struct ContextStatus # 当前 daemon 持有的暂停和工作线程状态，与持久化历史分开查询。
    │   ├── [x] struct CheckpointRecord # 保存所属 Trace、来源 save、快照 ID、失效和删除状态。
    │   ├── [x] struct OperationRecord # 保存 save 或 restore 的阶段、结果、失败原因、目标、边界信息和失效的 checkpoint。
    │   ├── [x] struct Failure # 记录中保存的失败原因：code 和 message。
    │   ├── [x] struct SnapshotContext # 一次调用针对谁的 checkpoint：现在是一个 Trace；以后一个 Trace 里有多个 session 时在这里加字段，接口签名不变。
    │   └── [x] enum FilterMode # checkpoint 列表怎么选：全部、最新若干个，或某个创建时间范围内的；都按创建顺序返回，已删除的也列出。
    ├── [x] scope.rs # 按最内层归属划分嵌套 scope，把子 scope 的根目录加入父 scope 的排除路径，并按 backend 分组；相对排除路径按 scope 根目录解析；拒绝重复 id、相同根目录、非绝对路径和根目录之外的排除路径。
    │   ├── [x] struct EffectiveScope # 解决嵌套之后的 scope。
    │   └── [x] struct BackendGroup # 一个 backend 收到的有效 scope，按配置中首次出现的顺序排列。
    ├── [x] coordinator/ # 按 SnapshotContext 调度 save、restore、删除与 disconnect 并更新历史；公开方法分为命令、查询和事件三组，都以 SnapshotContext 指明对象。
    │   ├── [x] mod.rs # 协调器及其端口。
    │   │   ├── [x] struct CheckpointCoordinator # 持有记录库的写连接与只读连接、factory 和每个 Trace 的状态；只有查询使用只读连接。
    │   │   └── [x] trait CompletionSink # 每个任务结束后通知 daemon，并带出可以越过边界的 hold；launch 的结果用于回复 TrackAdd。
    │   ├── [x] commands.rs # 命令：launch、save、restore、force、delete、disconnect、resume。在记录库锁内检查 Trace 状态并记录请求后返回，耗时工作排入 Trace 队列；恢复等到下一个边界，上次恢复未成功仍保持 Trace 时不等；Trace 结束或 disconnect 之后不能 save 或 restore。
    │   │   └── [x] struct LaunchRequest # Trace 所有者、停在启动屏障内的根进程和 scope 配置。
    │   ├── [x] queries.rs # 查询：checkpoint 列表、operation 和 Trace 所有者经只读连接读取；当前暂停和工作线程状态读内存。均在调用线程执行，不排队，也不等记录库锁。
    │   ├── [x] events.rs # 事件：LLM 请求边界和 Trace 结束。边界处有等待的恢复就执行恢复，Trace 被保持时继续等待，正在 disconnect 时直接放行，否则执行 save；Trace 结束时执行等待中的恢复，然后 disconnect，或重试失败的 disconnect；daemon 停止时 disconnect 运行中的 Trace 并等待工作结束。
    │   │   └── [x] enum BoundaryAction # 边界处的 Agent 立即继续，或等其 hold 随完成通知返回。
    │   ├── [x] scheduler.rs # 每个 SnapshotContext 一个工作线程，有任务时启动、队列空时退出，不同 context 并行；保存初始 checkpoint 是否完成、backend、暂停和待补记结果，调用 backend 时不加锁。
    │   ├── [x] recover.rs # daemon 启动时收尾：结束上次中断的 operation，补上 Trace 缺失的结束时间，把没有 disconnect 或 disconnect 失败的 Trace 记为无法重试、其 checkpoint 记为已删除且释放失败，没有回音的释放也记为失败；不调用 backend。
    │   └── [x] jobs/ # 在 Trace 工作线程上执行的工作：调用 backend、控制进程并记录结果。
    │       ├── [x] launch.rs # 由 factory 创建 backend 并 connect，建立 Trace 记录后执行初始 save；没有得到 checkpoint 时 disconnect 并让 launch 失败。
    │       ├── [x] save.rs # 调用 snapshot，成功才创建 checkpoint；记录失败时删除这个快照。
    │       ├── [x] restore.rs # 恢复与 resume：暂停仍存活的进程、调用 backend 并记录失效的 checkpoint；成功时继续进程并放行之前保持的边界，失败时连同本次边界保持 Trace；继续进程成功前保留暂停状态，resume 时一并放行。
    │       ├── [x] records.rs # 保留写库失败的恢复与 disconnect 结果，后续命令补记结果及失效、释放信息，不重复已完成的 backend 调用。
    │       ├── [x] release.rs # 删除 checkpoint 后释放它的快照并记录结果。
    │       └── [x] disconnect.rs # 在此前排队的工作完成后删除剩下的 checkpoint，disconnect 并记录结果，失败时保留 backend 等待重试；launch 失败和 daemon 停止时同时记录结束时间，手动 disconnect 保留运行状态；之后放行恢复失败时保持的进程和边界。
    └── [x] ports.rs # 协调器依赖的外部能力。
        ├── [x] trait CheckpointCatalog # 读写 Trace 记录、checkpoint 和 operation；结束 save 与创建 checkpoint、结束 restore 与标记失效各在一个事务内完成。
        ├── [x] trait CatalogReader # 记录库的只读连接，供查询使用，查询不等写入持有的锁。
        └── [x] trait ProcessControl # 暂停 Trace 中仍存活的进程，只继续由 checkpoint 暂停的进程。
```
