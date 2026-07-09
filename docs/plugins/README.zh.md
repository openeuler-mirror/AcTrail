# AcTrail 插件文档

本目录是 AcTrail 插件系统的文档入口。插件使用者通常先阅读操作手册；插件作者需要同时阅读 ABI 文档和示例代码。

## 文档入口

| 文档 | 适用对象 | 内容 |
| --- | --- | --- |
| [插件操作手册](operator-manual.zh.md) | 运维和插件使用者 | 插件加载、卸载、列表、状态查看、manifest、插件配置和授权。 |
| [插件 ABI 文档索引](abi/README.zh.md) | 插件作者 | WASM core module、WIT component、观测消费者、控制决策和 LLM codec ABI。 |

## ABI 文档

| 文档 | 内容 |
| --- | --- |
| [WASM Core Module ABI](abi/wasm-core-module.zh.md) | 普通 WASM module 的内存、导出函数和 hostcall 约定。 |
| [观测消费者 ABI](abi/observation-consumer.zh.md) | `observation-consumer` 插件的输入、返回值和消费语义。 |
| [控制决策 ABI](abi/control-decider.zh.md) | `control-decider` 插件的同步决策输入、返回值、hostcall 和性能约束。 |
| [LLM Codec ABI](abi/llm-codec.zh.md) | `llm-codec` 插件的 request/SSE 解码入口、输出格式和失败回退语义。 |

## 示例入口

插件示例位于 [examples/plugins](../../examples/plugins/README.zh.md)。示例目录按运行形态和插件用途分类：

| 示例分类 | 内容 |
| --- | --- |
| `builtin/` | 内置插件示例，例如 OTEL JSONL 观测消费者。 |
| `wasm-legacy/` | WASM core module 示例，适合阅读底层内存 ABI 和 `.wat` 写法。 |
| `wit-component/` | WIT component 示例，适合正式插件开发和 Rust component 插件参考。 |

## 阅读顺序

1. 需要加载或管理插件：先读 [插件操作手册](operator-manual.zh.md)。
2. 需要编写插件：先读 [插件 ABI 文档索引](abi/README.zh.md)，再按插件类型阅读具体 ABI。
3. 需要参考可运行插件：读 [examples/plugins](../../examples/plugins/README.zh.md)，再进入对应示例目录。
