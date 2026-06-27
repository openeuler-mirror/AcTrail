# 内置 OTEL JSONL 观测插件

类别：内置观测消费者。

这个示例使用 `runtime = "builtin"` 和 `id = "otel-jsonl"`。它展示了如何通过插件生命周期加载 AcTrail 内置的 OTEL JSONL 输出能力，并把输出路径、队列容量等业务参数放在插件自己的配置文件中。

文件：

- `plugin.toml`：插件 manifest。
- `config.toml`：插件自己的 TOML 配置。
- `otel-jsonl.plugin-config.v1`：`schema_ref` 指向的 JSON Schema。

加载示例：

```bash
target/release/actraild --config operator.conf plugin load \
  --manifest examples/plugins/builtin/otel-jsonl/plugin.toml \
  --plugin-config examples/plugins/builtin/otel-jsonl/config.toml \
  --instance dynamic.otel-jsonl
```

查看状态：

```bash
target/release/actraild --config operator.conf plugin status \
  --instance dynamic.otel-jsonl
```

也可以写入 `operator.conf`，让 daemon 启动时自动加载：

```toml
[plugins.startup]
enabled = true
failure_policy = "fail-fast"

[[plugins.startup.load]]
instance = "live-otel"
enabled = true
failure_policy = "continue"
manifest = "examples/plugins/builtin/otel-jsonl/plugin.toml"
plugin_config = "examples/plugins/builtin/otel-jsonl/config.toml"
host_grants = []
```
