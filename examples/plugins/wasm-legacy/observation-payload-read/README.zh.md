# WASM Core Module Payload 读取插件

类别：WASM core module 观测消费者。

这个示例声明 `host.capabilities = ["payload-read"]`，插件可以通过 hostcall 读取 AcTrail 为当前观测 batch 保留的 payload 数据。读取范围由 manifest 中的 `hostcall_limits.payload.*` 和加载时的 `--grant` 决定。

文件：

- `plugin.toml`：插件 manifest。
- `payload-read.wat`：WebAssembly Text artifact。

相关 ABI 说明见 [WASM Core Module ABI](../../../../docs/plugins/abi/wasm-core-module.zh.md) 和 [观测消费者 ABI](../../../../docs/plugins/abi/observation-consumer.zh.md)。

常用授权方式：

```bash
--grant payload-read
--grant payload-read:source=syscall
```
