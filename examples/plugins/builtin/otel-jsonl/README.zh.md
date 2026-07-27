# 内置 OTEL JSONL 观测插件

类别：内置观测消费者。

这个示例使用 `runtime = "builtin"` 和 `id = "otel-jsonl"`。它展示了如何通过插件生命周期加载 AcTrail 内置的 OTEL JSONL 输出能力，并把输出路径、队列容量等业务参数放在插件自己的配置文件中。release 安装器会把这个完整插件包安装到 `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}/otel-jsonl`，使它出现在 Web 的 **Plugin candidates** 中，但不会自动加载。

文件：

- `otel-jsonl.plugin.toml`：插件 manifest；文件名符合插件目录发现约定。
- `otel-jsonl.config.toml`：插件自己的 TOML 配置。
- `otel-jsonl.plugin-config.v1`：`schema_ref` 指向的 JSON Schema。

加载示例：

```bash
target/release/actraild --config operator.conf plugin load \
  --manifest examples/plugins/builtin/otel-jsonl/otel-jsonl.plugin.toml \
  --plugin-config examples/plugins/builtin/otel-jsonl/otel-jsonl.config.toml \
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
manifest = "examples/plugins/builtin/otel-jsonl/otel-jsonl.plugin.toml"
plugin_config = "examples/plugins/builtin/otel-jsonl/otel-jsonl.config.toml"
host_grants = []
```

## 通过 Web 启用

1. 确认 `[plugins.discovery].directory` 指向安装器使用的插件根目录。
2. 打开 Web 的 **Plugins** 工作区并点击 **Refresh**。
3. 在 **Plugin candidates** 中找到 `otel-jsonl`。
4. 需要时先编辑安装目录中的 `otel-jsonl.config.toml`，再点击 **Configure & load**。
5. 使用实例 ID `live-otel` 或其他非空且未占用的名称完成加载。

加载后，该候选会进入 **Loaded plugin instances**。实例状态中的 `observed_records` 应随运行中的 semantic action 增长；`dropped_records` 和 `last_error` 用于发现队列拥塞或文件写入错误。
