# WASM Core Module LLM Codec No-op 插件

类别：WASM core module LLM codec。

这个示例使用手写的 `noop.wat` 作为插件 artifact。插件声明 `role = "llm-codec"`，但对所有 request body 和 SSE event data 都返回 `{"status":"no_match"}`。它适合用来理解 LLM codec 插件的 manifest、WASM core module 导出函数和失败回退语义。

文件：

- `plugin.toml`：插件 manifest。
- `noop.wat`：WebAssembly Text artifact。

加载示例：

```bash
target/release/actraild --config operator.conf plugin load \
  --manifest examples/plugins/wasm-legacy/llm-codec-noop/plugin.toml \
  --instance llm-codec.noop
```

查看状态：

```bash
target/release/actraild --config operator.conf plugin status \
  --instance llm-codec.noop
```

预期状态字段：

```text
purpose=llm-codec
runtime=wasm
state=active
last_error=none
```

相关 ABI 说明见 [WASM Core Module ABI](../../../../docs/plugins/abi/wasm-core-module.zh.md) 和 [LLM Codec ABI](../../../../docs/plugins/abi/llm-codec.zh.md)。
