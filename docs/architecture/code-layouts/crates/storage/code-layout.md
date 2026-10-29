状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/storage/ # 主观测存储契约、后端选择，以及 SQLite 与 noop 实现
├── [x] core/ # 汇集主存储能力和面向查询的分页模型
│   └── [x] src/ # 后端接口、事务和控制状态存储接口
│       ├── [x] lib.rs # 导出主存储能力与查询类型
│       ├── [x] backend.rs # 聚合 trace、事件、语义、payload 和控制状态的存取能力
│       │   ├── [x] trait StorageBackend # 提供记录保存、历史读取、保留策略和事务入口
│       │   └── [x] enum StorageOpenMode # 选择只读或读写打开方式
│       ├── [x] error.rs # 定义存储失败与属性补充拒绝信息。
│       │   └── [x] struct StorageError # 保存存储失败信息。
│       ├── [x] transaction.rs # 主存储事务接口
│       │   └── [x] trait StorageTransaction # 控制当前事务提交和回滚
│       ├── [x] cgroup_v1_bindings.rs # 定义 v1 绑定控制状态存取接口
│       │   └── [x] trait CgroupV1BindingStore # 创建、读取、失效和关闭 v1 绑定记录
│       ├── [x] timeline.rs # 时间线游标、范围与分页结果
│       │   ├── [x] struct TimelineQuery # 描述时间线查询方向和范围
│       │   └── [x] struct TimelinePage # 返回带游标的时间线记录
│       ├── [x] trace_page.rs # trace 列表分页与数量汇总
│       │   ├── [x] struct TracePageQuery # 保存 trace 筛选与分页条件
│       │   └── [x] struct TraceCounts # 汇总 trace 关联数据数量
│       ├── [x] action_content.rs # HTTP 等行为正文的读取模型
│       │   └── [x] struct HttpContentQuery # 保存正文读取范围与预算
│       ├── [x] idle.rs # 空闲区间查询模型
│       │   └── [x] struct IdleInterval # 描述一个空闲时间区间
│       └── [x] payload_retention.rs # 明文保留预算
│           └── [x] struct PayloadRetentionLimits # 限制 trace 保存的明文范围
├── [x] factory/ # 解析存储配置并创建选定后端
│   └── [x] src/ # 后端配置、解析和实例构造
│       ├── [x] lib.rs # 导出 open_storage_backend 与后端配置类型
│       ├── [x] config.rs # 表达存储后端选择和专属配置
│       │   ├── [x] enum StorageBackendKind # 标识 SQLite 或 noop 后端
│       │   └── [x] enum StorageConfig # 保存所选后端的完整配置
│       ├── [x] parser.rs # 解析 operator 传入的存储子配置
│       └── [x] builder.rs # open_storage_backend 按配置和打开模式构造 StorageBackend
└── [x] adapters/ # 主存储契约的具体后端
    ├── [x] noop/ # 跳过观测持久化，保留运行所需控制状态
    │   └── [x] src/ # 内存控制状态与事务
    │       ├── [x] storage.rs # 实现不保留观测的 StorageBackend
    │       │   └── [x] struct NoOpStorage # 提供空观测存储和必要的内存控制记录
    │       ├── [x] backend.rs # 实现空观测存储接口。
    │       ├── [x] resource_state.rs # 保存资源准入、关闭和外部绑定控制记录
    │       │   └── [x] struct ResourceState # 持有可参与事务的控制状态
    │       ├── [x] transaction.rs # 管理内存控制记录的事务边界
    │       │   └── [x] struct NoOpTransaction # 提交或回滚当前内存状态变更
    │       └── [x] cgroup_v1.rs # 保留准入及完成所需控制状态，参与内存事务
    └── [x] sqlite/ # 保存主观测、语义记录和控制状态的 SQLite 后端
        └── [x] src/ # 连接、记录编码、写入、查询和清理
            ├── [x] lib.rs # 创建数据库连接并组合字典、批次与租约状态
            │   └── [x] struct SqliteStorage # open/open_read_only 打开数据库；持有连接、明文保留状态、字典与事件块写入器
            ├── [x] backend.rs # 将 StorageBackend 方法适配到 SQLite 记录和查询实现
            ├── [x] transaction.rs # 创建数据库写事务并处理提交与回滚
            │   └── [x] struct SqliteWriteTransaction # 管理 SQLite 事务结束边界
            ├── [x] config/ # SQLite 存储专属配置
            │   └── [x] sqlite.rs # 数据库路径、等待预算、事件布局与压缩参数
            │       ├── [x] struct SqliteStorageConfig # 持有 SQLite 后端全部打开选项
            │       └── [x] enum EventRecordLayout # 选择事件记录的持久化布局
            ├── [x] records/ # 原始事件与明文记录的编码和压缩
            │   ├── [x] payload_dictionary.rs # 复用 trace 内重复事件载荷
            │   │   └── [x] struct EventPayloadDictionary # 管理有界载荷字典缓存
            │   ├── [x] path_dictionary.rs # 将重复路径保存为字典引用
            │   │   └── [x] struct PathInterner # 管理 trace 路径驻留和缓存
            │   ├── [x] event_codec/ # 原始事件载荷的编码实现
            │   │   ├── [x] mod.rs # 定义编码接口和载荷块模型
            │   │   │   └── [x] trait EventPayloadCodec # 编码和解码原始事件载荷
            │   │   ├── [x] bincode_codec.rs # Bincode 事件载荷编码
            │   │   │   └── [x] struct BincodeCodec # 实现 Bincode 编解码
            │   │   └── [x] manual.rs # 显式布局的事件载荷编码
            │   │       └── [x] struct ManualCodec # 实现手工事件编解码
            │   └── [x] event_record_blocks/ # 将事件按数量与字节预算合并为压缩块
            │       └── [x] builder.rs # 累积当前事件块并刷新写入
            │           └── [x] struct EventRecordBlockWriter # 管理事件块大小与待写批次
            ├── [x] query/ # trace 列表与关联数量查询
            │   └── [x] trace_page.rs # 执行 trace 分页和过滤
            │       └── [x] struct TracePageReader # 借用数据库连接读取 trace 页面
            ├── [x] semantic_actions/ # 语义行为、链接、LLM 正文和轨迹关系的持久化
            │   ├── [x] codebook/ # 管理语义种类与关系角色的整数编码。
            │   ├── [x] store/ # 保存动作及其增量状态。
            │   │   └── [x] state.rs # 保存动作状态与属性更新。
            │   ├── [x] query.rs # 查询语义动作与关系记录。
            │   ├── [x] root_tree.rs # 查询展示根节点与子项信息。
            │   ├── [x] root_tree/ # 组织展示根节点查询。
            │   └── [x] navigation.rs # 查询动作展示路径与位置。
            ├── [x] payload/ # 明文段保存、读取与保留预算
            ├── [x] alerts/ # 通用插件告警定义、记录和查询
            ├── [x] retention.rs # trace 保留期限、租约保护和分批清理
            ├── [x] resource_scopes.rs # 保存托管资源范围的恢复与关闭记录
            ├── [x] external_cgroups.rs # 保存外部 cgroup 关联与生命周期
            └── [x] cgroup_v1/ # v1 绑定身份、失效与关闭状态存储
                ├── [x] bindings.rs # 保存 v1 绑定身份、失效原因与关闭状态
                │   └── [x] struct BindingRow # 将 SQLite 记录转换为 v1 绑定模型
                ├── [x] mod.rs # 声明 v1 绑定存储模块
                └── [x] schema.sql # 定义 v1 绑定控制状态表与存活绑定索引
```
