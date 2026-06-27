# WASM Core Module 网络拒绝插件

类别：WASM core module 控制决策插件。

这个示例用于网络动作控制。当 `[network_control]` 规则精确命中 TCP `connect <ip:port>` 目标时，daemon 会在继续 seccomp notification 前调用插件。该示例返回拒绝决策，使目标连接收到 `EPERM`。

文件：

- `plugin.toml`：插件 manifest。
- `deny-network.wat`：WebAssembly Text artifact。

相关 ABI 说明见 [WASM Core Module ABI](../../../../docs/plugins/abi/wasm-core-module.zh.md) 和 [控制决策 ABI](../../../../docs/plugins/abi/control-decider.zh.md)。
