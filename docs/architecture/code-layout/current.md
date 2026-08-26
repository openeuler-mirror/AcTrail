# 当前代码布局

> 本文展示组合根、核心运行时、契约和适配器的当前代码归属。

工作空间采用 app/core/contract/adapter 分层。**组合根**是创建具体适配器，并将它们连接到核心运行时端口的外层应用模块。

```text
crates/
  apps/                         组合根和二进制程序
  core/                         运行时协调和领域行为
  contracts/                    跨层值与端口
  adapters/                     eBPF、传输、存储、控制和导出适配器
  plugins/                      内置插件和 WebAssembly（WASM）插件实现
  tools/                        诊断和探针发现
```

`apps` 组装核心运行时和适配器。`core` 模块依赖 `contracts`；`adapters` 实现运行时端口或契约。公开导出集中在模块门面。

重要的组合根包括 `crates/apps/daemon`、`crates/apps/control`、`crates/apps/web`、`crates/apps/viewer`、`crates/apps/sb` 和 `crates/apps/vsock_gateway`。

执行隔离路径单独说明，因为它的 Guest 采集器、链路契约、网关、存储和插件有意独立于主观测路径。
