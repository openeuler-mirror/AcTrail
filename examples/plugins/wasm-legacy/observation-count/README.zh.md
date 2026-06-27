# WASM Core Module 观测计数插件

类别：WASM core module 观测消费者。

这个示例使用手写的 `count.wat` 作为插件 artifact。插件消费 AcTrail 推送的观测 batch，并报告已处理记录数。它适合用来理解 WASM core module ABI 的最小导出函数和内存约定。

文件：

- `plugin.toml`：插件 manifest。
- `config.toml`：插件自己的 TOML 配置。
- `wasm-observation-count.v1`：`schema_ref` 指向的 JSON Schema。
- `count.wat`：WebAssembly Text artifact。

相关 ABI 说明见 [WASM Core Module ABI](../../../../docs/plugins/abi/wasm-core-module.zh.md) 和 [观测消费者 ABI](../../../../docs/plugins/abi/observation-consumer.zh.md)。
