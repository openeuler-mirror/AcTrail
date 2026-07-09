# AcTrail 插件示例

本目录提供可阅读、可复制的 AcTrail 插件示例。每个子目录都是一个独立插件示例，包含插件 manifest、必要的插件配置、插件 artifact，以及需要时的源码。

## 目录分类

| 目录 | 运行时 / ABI | 插件用途 | 示例内容 |
| --- | --- | --- | --- |
| `builtin/otel-jsonl` | `builtin` | `observation-consumer` | 内置 OTEL JSONL 观测消费者，使用插件自己的 TOML 配置。 |
| `wasm-legacy/observation-count` | `wasm` core module | `observation-consumer` | 最小观测消费者 ABI，使用手写 `.wat` 模块统计观测记录。 |
| `wasm-legacy/observation-env-read` | `wasm` core module | `observation-consumer` | 观测插件通过显式授权读取指定环境变量。 |
| `wasm-legacy/observation-payload-read` | `wasm` core module | `observation-consumer` | 观测插件通过显式授权读取保留的 payload 数据。 |
| `wasm-legacy/llm-codec-noop` | `wasm` core module | `llm-codec` | 最小 LLM codec ABI，始终返回 `no_match`，用于验证加载和失败回退语义。 |
| `wasm-legacy/llm-codec-qoder` | `wasm` core module | `llm-codec` | Qoder CLI LLM request/SSE codec，展示特定 agent 解码逻辑如何只存在于插件内。 |
| `wasm-legacy/control-graylist` | `wasm` core module | `control-decider` | fanotify 灰名单文件访问同步决策。 |
| `wasm-legacy/control-command-deny` | `wasm` core module | `control-decider` | 显式命令执行策略命中后拒绝执行。 |
| `wasm-legacy/control-network-deny` | `wasm` core module | `control-decider` | 显式 TCP connect 策略命中后拒绝连接。 |
| `wit-component/observation-read-config` | `wasm` WIT component | `observation-consumer` | Rust 编写的 component 观测插件，按需读取插件配置。 |
| `wit-component/observation-payload-read` | `wasm` WIT component | `observation-consumer` | Rust 编写的 component 观测插件，读取 payload 数据。 |
| `wit-component/control-graylist` | `wasm` WIT component | `control-decider` | Rust 编写的 component 控制插件，处理 fanotify 灰名单决策。 |
| `wit-component/control-hostcalls` | `wasm` WIT component | `control-decider` | Rust 编写的 component 控制插件，使用 context 和 file-policy hostcall。 |
| `wit-component/file-policy-dynamic` | `wasm` WIT component | `control-decider` | Rust 编写的动态文件策略插件，通过 `plugin cmd` 管理 allow/deny/gray 规则。 |

## 插件文件模型

每个插件示例至少包含：

- `plugin.toml`：插件 manifest。
- 插件 artifact：例如 `.wat` 或 `.wasm`。
- `README.zh.md`：该插件的说明。

如果插件声明了 `plugin_config.required = true`，目录中还会包含 `config.toml`。如果 manifest 使用了 `plugin_config.schema_ref`，目录中还会包含对应的 JSON Schema 文件。

## WASM Core Module 示例

`wasm-legacy` 下的示例直接使用 `.wat` 作为 `runtime.wasm.artifact_path`。这些示例适合阅读 AcTrail WASM core module ABI，因为 `.wat` 可以直接看到导出函数、内存布局和 hostcall 调用方式。

这类示例通常不需要单独编译，加载时 manifest 直接引用 `.wat`：

```toml
[general]
runtime = "wasm"

[runtime.wasm]
artifact_path = "count.wat"
```

WASM core module 的固定导出、可选导出和输入写入流程见 [WASM Core Module ABI](../../docs/plugins/abi/wasm-core-module.zh.md)。观测消费者语义见 [观测消费者 ABI](../../docs/plugins/abi/observation-consumer.zh.md)，控制决策语义见 [控制决策 ABI](../../docs/plugins/abi/control-decider.zh.md)，LLM codec 语义见 [LLM Codec ABI](../../docs/plugins/abi/llm-codec.zh.md)。

## WIT Component 示例

`wit-component` 下的示例使用 Rust 编写插件源码，并提交已编译的 `.wasm` component artifact。

通用构建方式：

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wit-component/<插件目录>/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/<crate-name>.wasm ../<artifact-name>.wasm
```

具体 crate 名称和 artifact 名称见各插件子目录的 `README.zh.md`。

## 加载插件

通用加载命令：

```bash
target/release/actraild --config operator.conf plugin load \
  --manifest examples/plugins/<分类>/<插件目录>/plugin.toml \
  --instance my.instance
```

如果插件需要自己的配置：

```bash
target/release/actraild --config operator.conf plugin load \
  --manifest examples/plugins/<分类>/<插件目录>/plugin.toml \
  --plugin-config examples/plugins/<分类>/<插件目录>/config.toml \
  --instance my.instance
```

如果插件声明了 host capability，还需要对应的 `--grant`。例如：

```bash
--grant env-read:ACTRAIL_PLUGIN_SECRET
--grant payload-read:source=syscall
--grant context-query
--grant file-access.current-match-get
```

ABI 文档入口见 [插件 ABI 文档索引](../../docs/plugins/abi/README.zh.md)。完整操作说明见 [插件操作手册](../../docs/plugins/operator-manual.zh.md)。
