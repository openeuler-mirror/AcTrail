# 插件 API 参考

> 本文按插件用途和运行形态列出插件作者必须实现的接口与精确数据约定。

本目录说明 AcTrail 插件与 daemon 之间的调用约定。先按插件用途选择功能层，再按运行
形态判断是否还需要 core module 承载层。

## 术语

| 术语 | 含义 |
| --- | --- |
| 插件包 | 一组可部署资产，通常包含 manifest、artifact、业务配置和配置 schema。 |
| manifest | 描述插件 ID、API 版本、角色、运行时、artifact、资源限制和 capability 请求的 TOML 文件。 |
| role | 插件用途：观测消费、控制决策或 LLM codec。role 决定宿主调用哪个功能接口。 |
| runtime | 插件运行形态：WASM core module、WIT component 或编译进 daemon 的 builtin。 |
| ABI | daemon 与插件在二进制边界上的精确约定，包括导出名称、参数、内存布局、编码和返回值。 |
| capability | manifest 声明插件需要使用的一类宿主能力。声明本身不授予权限。 |
| grant | 管理员在加载时对 capability 给出的实际授权，可进一步限制变量名、路径或规则范围。 |
| hostcall | 插件调用 daemon 能力的受控函数；只有 ABI 提供且 grant 允许的 hostcall 才能成功。 |
| WASM core module | 直接导出函数和线性内存的普通 WebAssembly module。插件作者负责承载 ABI。 |
| WIT component | 使用 WebAssembly Component Model 和 WIT 结构化接口的 component。工具链负责底层 lowering/lifting。 |
| builtin | 编译进 daemon 的 Rust 实现；仍通过插件生命周期加载，但不实现 WASM ABI。 |

## 文档关系

| 文档 | 层级 | 适用对象 | 说明 |
| --- | --- | --- | --- |
| [WASM Core Module ABI](wasm-core-module.md) | 承载层 | 使用普通 WebAssembly module 的插件 | 说明 `memory`、`actrail_alloc`、可选 `actrail_plugin_init`，以及 AcTrail 如何把输入数据写入插件内存。 |
| [观测消费者 ABI](observation-consumer.md) | 功能层 | `observation-consumer` 插件 | 说明观测 batch 的消费入口、输入语义和返回约定。 |
| [控制决策 ABI](control-decider.md) | 功能层 | `control-decider` 插件 | 说明同步治理决策入口、请求语义、返回码和 `once` / `reusable`。 |
| [LLM Codec ABI](llm-codec.md) | 功能层 | `llm-codec` 插件 | 说明 LLM request body 和 SSE event data 的可选解码入口、输出 JSON 和失败回退语义。 |

## 按插件类型阅读

| 插件类型 | 必读文档 |
| --- | --- |
| WASM core module 观测消费者 | [WASM Core Module ABI](wasm-core-module.md) + [观测消费者 ABI](observation-consumer.md) |
| WASM core module 控制决策插件 | [WASM Core Module ABI](wasm-core-module.md) + [控制决策 ABI](control-decider.md) |
| WASM core module LLM codec 插件 | [WASM Core Module ABI](wasm-core-module.md) + [LLM Codec ABI](llm-codec.md) |
| WIT component 观测消费者 | [观测消费者 ABI](observation-consumer.md) |
| WIT component 控制决策插件 | [控制决策 ABI](control-decider.md) |
| 内置插件 | 先看具体插件说明；内置插件不需要实现 WASM ABI。 |

## 作者导航

1. 插件作者首先确认插件用途：观测消费、控制决策还是 LLM codec。
2. 插件作者随后确认运行形态：WASM core module、WIT component 或 builtin。
3. WASM core module 作者先读承载层 ABI，再读对应功能层 ABI。
4. WIT component 作者直接读对应功能层 ABI；component model 处理底层导出。
5. 插件作者根据 manifest 的 capability 请求设计最小 grant；操作侧见 [管理插件](../../operations/plugins/manage.md)。

Rust 插件可以依赖 `actrail_plugin_abi` crate 复用稳定 ABI 常量，例如 `actrail_plugin_abi::control::context::CURRENT_DECISION` 和 `actrail_plugin_abi::control::query::DECISION_SUMMARY`。AcTrail 宿主侧也从同一 crate 引用这些值，避免宿主和示例插件各自维护一份字符串。

插件宿主、三种运行形态及调用链见
[插件运行时架构](../../architecture/components/plugin-runtime.md)。
