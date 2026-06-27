# WASM Core Module 灰名单文件控制插件

类别：WASM core module 控制决策插件。

这个示例用于 fanotify 文件访问控制。当 AcTrail 文件策略命中 `gray sync-plugin` 规则时，daemon 会同步调用该插件，由插件返回 `allow` 或 `deny`。

文件：

- `plugin.toml`：插件 manifest。
- `config.toml`：插件自己的 TOML 配置。
- `wasm-file-graylist-allow.v1`：`schema_ref` 指向的 JSON Schema。
- `allow-on-gray.wat`：WebAssembly Text artifact。

相关 ABI 说明见 [WASM Core Module ABI](../../../../docs/plugins/abi/wasm-core-module.zh.md) 和 [控制决策 ABI](../../../../docs/plugins/abi/control-decider.zh.md)。
