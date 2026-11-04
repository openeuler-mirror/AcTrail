状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
./ # AcTrail 观测、治理与开发工具
├── [x] Cargo.toml # Rust 工作空间成员、共享依赖和构建选项
├── [x] crates/ # 应用、领域运行时、契约与平台适配
├── [ ] integrations/ # 集成源码；按目标环境可用性编译为动态库
│   └── [ ] checkpoint/ # checkpoint 集成
│       ├── [ ] filesystem/ # 文件系统快照集成
│       │   ├── [ ] btrfs/ # 编译生成 libactrail_ckpt_fs_btrfs.so
│       │   └── [ ] agentns/ # 编译生成 libactrail_ckpt_fs_agentns.so
│       └── [ ] context/ # Agent 上下文快照集成
│           └── [ ] xiaoo/ # 编译生成 libactrail_ckpt_cxt_xiaoo.so
├── [x] scripts/ # 性能测量和开发辅助工具
│   └── [x] bench/ # 按整体运行和采集链路组织性能测量
├── [x] tests/ # 真实 Agent、部署和用户操作的端到端验证
├── [x] deploy/ # 各部署模式的运行资源与启动配置
└── [x] examples/ # 接入和使用示例
```
