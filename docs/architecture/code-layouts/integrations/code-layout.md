状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[/] integrations/ # 集成源码；按目标环境与依赖可用性编译为独立动态库
└── [/] checkpoint/ # 实现 contracts/checkpoint 的同步 ABI 接口
    ├── [ ] filesystem/ # 文件系统快照集成
    │   ├── [ ] btrfs/ # Btrfs 集成源码 crate
    │   │   ├── [ ] Cargo.toml # cdylib；编译生成 libactrail_ckpt_fs_btrfs.so
    │   │   └── [ ] src/ # Btrfs 后端实现
    │   │       └── [ ] lib.rs # 导出共享 ABI 约定的动态库入口
    │   └── [ ] agentns/ # AgentNS 集成源码 crate
    │       ├── [ ] Cargo.toml # cdylib；编译生成 libactrail_ckpt_fs_agentns.so
    │       └── [ ] src/ # AgentNS 后端实现
    │           └── [ ] lib.rs # 导出共享 ABI 约定的动态库入口
    └── [x] context/ # Agent 上下文快照集成
        └── [x] xiaoo/ # Xiaoo 集成源码 crate
            ├── [x] Cargo.toml # cdylib；编译生成 libactrail_ckpt_cxt_xiaoo.so
            ├── [x] README.md # 配置、操作映射与真实 Agent 验证入口
            ├── [x] src/ # Xiaoo 上下文后端实现
            │   ├── [x] lib.rs # 导出共享 ABI 约定的动态库入口
            │   ├── [x] factory.rs # 解析整体配置并创建私有 backend
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

