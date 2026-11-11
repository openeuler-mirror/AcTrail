状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[/] integrations/ # 集成源码；按目标环境与依赖可用性编译为独立动态库
└── [/] checkpoint/ # 实现 contracts/checkpoint 的同步 ABI 接口
    ├── [x] filesystem/ # 文件系统快照集成
    │   ├── [x] btrfs/ # Btrfs 集成：每次保存为每个 scope 建一个只读快照，恢复时就地写回
    │   │   ├── [x] Cargo.toml # cdylib；编译生成 libactrail_ckpt_fs_btrfs.so
    │   │   ├── [x] config.example.json # 配置示例
    │   │   ├── [x] src/ # Btrfs 后端实现
    │   │   │   ├── [x] lib.rs # 导出动态库入口；检查 scope 是 Btrfs 子卷的根且在允许的工作区之下，拒绝排除路径和已有的嵌套子卷，保存、恢复、删除快照，disconnect 时删除全部快照
    │   │   │   │   └── [x] struct BtrfsBackend # 实现 RestorableCheckpointBackend，初始化时把快照容器目录设为 0700，包含已有目录
    │   │   │   ├── [x] btrfs.rs # 子卷检查、只读快照的创建与删除、固定目录句柄，以及 snapshot_root 下按子卷 UUID 命名的保存与恢复锁
    │   │   │   ├── [x] sync.rs # 把只读快照就地复制回受保护子卷，子卷身份保持不变；先按挂载表跳过嵌套挂载点，再读取其他条目元数据，避免进入用户 FUSE 挂载
    │   │   │   └── [x] config.rs # 解析并校验配置
    │   │   └── [x] tests/ # 在回环 Btrfs 上直接驱动 backend；run-real.sh 以 root 准备环境
    │   └── [x] agentns/ # AgentNS 集成：经 RNS（当前的 AgentNS）按会话记录 Trace 进程的全部修改，保存即封存当前轮
    │       ├── [x] Cargo.toml # cdylib；编译生成 libactrail_ckpt_fs_agentns.so
    │       ├── [x] config.example.json # 配置示例
    │       ├── [x] src/ # AgentNS 后端实现
    │       │   ├── [x] lib.rs # 导出动态库入口；建立 RNS 会话并关联 Trace 根进程，保存即封存当前轮，恢复前封存尾部修改，记住没有做完的恢复请求；disconnect 直接丢弃整个区域
    │       │   │   └── [x] struct AgentNsBackend # 实现 RestorableCheckpointBackend
    │       │   ├── [x] library.rs # 从 librns.so 加载 RNS C SDK，结构体与 rns_api.h 对应
    │       │   └── [x] config.rs # 解析并校验配置
    │       └── [x] tests/ # 用替身 librns（fake_rns.c）驱动 backend
    └── [x] context/ # Agent 上下文快照集成
        └── [x] xiaoo/ # Xiaoo 集成源码 crate
            ├── [x] Cargo.toml # cdylib；编译生成 libactrail_ckpt_cxt_xiaoo.so
            ├── [x] README.md # 配置、操作映射与真实 Agent 验证入口
            ├── [x] src/ # Xiaoo 上下文后端实现
            │   ├── [x] lib.rs # 导出共享 ABI 约定的动态库入口
            │   ├── [x] factory.rs # 声明 Context 后端类型，解析整体配置并创建私有 backend
            │   ├── [x] config.rs # 集成配置解析与启动校验
            │   ├── [x] constants.rs # 可配置传输限制的默认值
            │   ├── [x] backend.rs # trace/session 绑定与五个 checkpoint 操作
            │   ├── [x] client.rs # 有超时与响应大小限制的同步 Unix socket 请求
            │   ├── [x] protocol.rs # Xiaoo JSON 请求与响应类型
            │   └── [x] error.rs # 配置、快照、传输与远端错误映射
            └── [x] tests/ # 真实 Xiaoo 动态库端到端验证
                ├── [x] driver.rs # 通过共享 ABI 加载动态库并调用后端
                └── [x] run_e2e.py # 新配置启动真实 Agent 并核验上下文恢复和生命周期
```
