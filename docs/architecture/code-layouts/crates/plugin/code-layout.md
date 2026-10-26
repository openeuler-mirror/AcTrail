状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/plugin/ # 宿主与插件共享的底层 ABI 常量
└── [x] abi/ # 可在 no_std 环境使用的控制协议标识
    └── [x] src/ # ABI 常量定义与模块导出
        ├── [x] lib.rs # 启用 no_std 并导出 control 模块
        └── [x] control.rs # 定义上下文句柄、查询名称、能力授权名称和二进制操作码
            ├── [x] mod context # 当前文件、命令和网络控制上下文句柄
            ├── [x] mod query # 决策摘要、规则和操作上下文的查询标识
            ├── [x] mod file_policy # 文件裁决、规则更新、应用模式及授权标识
            ├── [x] mod command_policy # 命令规则协议版本、裁决与更新操作码及授权标识
            ├── [x] mod network_policy # 网络规则协议版本、裁决与更新操作码及授权标识
            └── [x] mod subject_code # 文件、命令和网络裁决对象的类型码
```
