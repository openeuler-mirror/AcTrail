# WASM Core Module 环境变量读取插件

类别：WASM core module 观测消费者。

这个示例声明 `host.capabilities = ["env-read"]`，插件在运行时通过 hostcall 读取被授权的环境变量。它展示了 capability 声明和 `--grant env-read:NAME` 授权之间的关系。

文件：

- `plugin.toml`：插件 manifest。
- `env-read.wat`：WebAssembly Text artifact。

相关 ABI 说明见 [WASM Core Module ABI](../../../../docs/plugins/abi/wasm-core-module.zh.md) 和 [观测消费者 ABI](../../../../docs/plugins/abi/observation-consumer.zh.md)。

加载时需要授予具体环境变量名：

```bash
--grant env-read:ACTRAIL_PLUGIN_SECRET
```
