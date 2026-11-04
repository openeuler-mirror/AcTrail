状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[ ] crates/core/checkpoint/ # checkpoint 配置、创建与组合模块
├── [ ] Cargo.toml # 依赖 contracts/checkpoint 与动态库加载能力
└── [ ] src/ # 模块公共入口与私有实现
    ├── [ ] lib.rs # 导出 factory 与配置入口；backends 模块及内部类型保持私有
    ├── [ ] factory.rs # 按配置创建、加载和组装后端，返回统一 CheckpointBackend
    ├── [ ] config.rs # 模块配置；描述组合结构、动态库路径及对应集成配置
    └── [ ] backends/ # 仅供模块 factory 组装的内部实现
        ├── [ ] mod.rs # 声明并在模块内部导出后端
        ├── [ ] composite.rs # 实现 RestorableCheckpointBackend；按配置组合子后端并管理子快照关联
        └── [ ] dynamic.rs # 按配置加载 .so、校验 ABI 并取得统一后端对象
```

