状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[ ] integrations/ # 集成源码；按目标环境与依赖可用性编译为独立动态库
└── [ ] checkpoint/ # 实现 contracts/checkpoint 的同步 ABI 接口
    ├── [ ] filesystem/ # 文件系统快照集成
    │   ├── [ ] btrfs/ # Btrfs 集成源码 crate
    │   │   ├── [ ] Cargo.toml # cdylib；编译生成 libactrail_ckpt_fs_btrfs.so
    │   │   └── [ ] src/ # Btrfs 后端实现
    │   │       └── [ ] lib.rs # 导出共享 ABI 约定的动态库入口
    │   └── [ ] agentns/ # AgentNS 集成源码 crate
    │       ├── [ ] Cargo.toml # cdylib；编译生成 libactrail_ckpt_fs_agentns.so
    │       └── [ ] src/ # AgentNS 后端实现
    │           └── [ ] lib.rs # 导出共享 ABI 约定的动态库入口
    └── [ ] context/ # Agent 上下文快照集成
        └── [ ] xiaoo/ # Xiaoo 集成源码 crate
            ├── [ ] Cargo.toml # cdylib；编译生成 libactrail_ckpt_cxt_xiaoo.so
            └── [ ] src/ # Xiaoo 上下文后端实现
                └── [ ] lib.rs # 导出共享 ABI 约定的动态库入口
```

