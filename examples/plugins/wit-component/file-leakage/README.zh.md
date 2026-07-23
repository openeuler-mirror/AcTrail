# 文件泄露告警插件

这个官方 WIT Component 只观察成功的 `write`/`writev` 文件操作。运行期间，它在插件自己的有界状态中保存工作目录和额外允许目录之外的候选路径；trace 进入终态后，它检查这些候选文件是否仍然存在，并通过 AcTrail 的异步告警入口写入 `file.leakage` 告警。

告警不是 semantic action。插件提交的内容进入独立的 `alerts` 表；一次提交对应一条告警记录，不按 trace 或告警类型去重。告警提交携带 trace 的授权 token，因此可以在 trace 结束后提交，且不依赖 trace retention 的执行时机。

下面假设在仓库根目录执行命令。

## 前置条件

构建 release 二进制和插件：

```bash
cargo build --workspace --release
cargo build --release --target wasm32-wasip2 \
  --manifest-path examples/plugins/wit-component/file-leakage/Cargo.toml
```

文件观测使用 eBPF，启动 daemon 和执行示例时需要 root 权限。

## 1. 安装插件包

release 安装脚本会把完整插件包安装到 `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}/file-leakage`：

```bash
scripts/install-release.sh
```

安装只让插件可被发现，不会自动加载。也可以手动准备包目录：

```bash
export PLUGIN_DIR="$HOME/.actrail/plugins/file-leakage"
mkdir -p "$PLUGIN_DIR"
cp examples/plugins/wit-component/file-leakage/file-leakage.plugin.toml "$PLUGIN_DIR/"
cp examples/plugins/wit-component/file-leakage/file-leakage.config.json "$PLUGIN_DIR/"
cp examples/plugins/wit-component/file-leakage/file-leakage.config.v1.schema.json "$PLUGIN_DIR/"
cp examples/plugins/wit-component/file-leakage/file-leakage.payload.v1.schema.json "$PLUGIN_DIR/"
cp examples/plugins/wit-component/file-leakage/target/wasm32-wasip2/release/actrail_file_leakage_plugin.wasm "$PLUGIN_DIR/"
```

## 2. 准备 operator 配置

生成默认配置：

```bash
export CONFIG="$HOME/.config/actrail/operator.conf"
sudo target/release/actraild --config "$CONFIG" init --force
```

确认 capture profile 至少包含文件基础观测能力，并且插件发现目录与安装目录一致：

```toml
[capture]
capabilities = ["proc-lifecycle", "fs-access-basic"]

[plugins.discovery]
directory = "~/.actrail/plugins"

[plugins.startup]
enabled = false
load = []

[plugins.alerts]
queue_capacity = 1024
writes_per_cycle = 256
drain_timeout_ms = 30000
```

`plugins.alerts` 只配置异步告警写入队列。它不改变 trace 生命周期，也不阻止 retention 清理终态 trace。

## 3. 配置插件

默认 `file-leakage.config.json`：

```json
{
  "include_trace_working_directory": true,
  "additional_allowed_roots": [],
  "trace_state_max_count": 256,
  "candidate_max_count": 4096
}
```

字段含义：

| 字段 | 说明 |
| --- | --- |
| `include_trace_working_directory` | 把 trace 启动时的工作目录作为允许目录。 |
| `additional_allowed_roots` | 额外允许的绝对目录；其子路径也视为允许。 |
| `trace_state_max_count` | 单个插件实例同时保存候选状态的 trace 上限。 |
| `candidate_max_count` | 每个 trace 保存的不同候选路径上限。 |

所有允许目录必须是绝对路径。配置超过上限时插件明确报错，不会静默丢弃候选。

## 4. 启动服务并通过 Web 加载

先启动 daemon：

```bash
sudo target/release/actraild --config "$CONFIG" start
```

再在另一个终端启动 Web：

```bash
sudo target/release/actrailweb --config "$CONFIG"
```

在浏览器中打开 `http://127.0.0.1:18080`，进入 **Plugins** 页面。页面分为两个区域：

- **Plugin candidates**：`${HOME}/.actrail/plugins` 下已发现、尚未加载的插件候选；
- **Loaded plugin instances**：daemon 当前实际运行的插件实例。

点击 **Refresh** 重新扫描插件目录。在 **Plugin candidates** 中找到 `actrail.file-leakage`，展开条目并确认：

- manifest 和 config 路径均位于 `${HOME}/.actrail/plugins/file-leakage`；
- capabilities 包含 `trace-file-state-read` 和 `alert-write`；
- issue 显示 `none`。

点击候选右侧的加载按钮。加载成功后，该候选从 **Plugin candidates** 消失，并出现在 **Loaded plugin instances**。展开已加载实例，确认状态为 `active`、Host grants 包含 `trace-file-state-read` 和 `alert-write`、Last error 为 `none`。

无浏览器环境可以使用等价的 CLI 入口：

```bash
sudo target/release/actraild --config "$CONFIG" plugin load \
  --manifest "$HOME/.actrail/plugins/file-leakage/file-leakage.plugin.toml" \
  --plugin-config "$HOME/.actrail/plugins/file-leakage/file-leakage.config.json" \
  --grant trace-file-state-read \
  --grant alert-write \
  --instance actrail.file-leakage
```

通过 CLI 确认状态：

```bash
sudo target/release/actraild --config "$CONFIG" plugin status \
  --instance actrail.file-leakage
```

## 5. 执行和验证

准备一个 trace 工作目录，以及位于工作目录之外的文件：

```bash
export DEMO_DIR=/tmp/actrail-file-leakage-demo
mkdir -p "$DEMO_DIR/work"
rm -f "$DEMO_DIR/leaked.txt"
```

在工作目录中启动被跟踪进程，并写入外部文件：

```bash
cd "$DEMO_DIR/work"
sudo target/release/actrailctl --config "$CONFIG" launch \
  --name file-leakage-demo -- \
  /bin/sh -c "printf 'leaked\n' >> '$DEMO_DIR/leaked.txt'"
```

预期现象：

- trace 运行期间，插件只保存成功写操作产生的越界候选；
- trace 结束后，插件确认 `$DEMO_DIR/leaked.txt` 仍存在；
- daemon 异步向独立 `alerts` 表追加一条 `file.leakage` 告警。

默认 Web 地址为 `127.0.0.1:18080`。查看告警：

```bash
curl -s http://127.0.0.1:18080/api/alerts
```

响应中的告警应包含：

```json
{
  "producer_plugin_id": "actrail.file-leakage",
  "definition_key": "file-leakage",
  "kind": "file.leakage",
  "payload": {
    "residual_files": ["/tmp/actrail-file-leakage-demo/leaked.txt"]
  }
}
```

写入 trace 工作目录内的文件不会触发告警；只创建但没有执行 `write`/`writev` 的文件不会成为候选；越界文件在 trace 结束前被删除也不会触发告警。

## 6. 卸载

在 **Plugins** 页面的 **Loaded plugin instances** 中找到 `actrail.file-leakage`，点击右侧卸载按钮。卸载成功后，该实例从已加载列表消失；点击 **Refresh** 后，插件重新出现在候选列表。

无浏览器环境可以使用等价的 CLI 入口：

```bash
sudo target/release/actraild --config "$CONFIG" plugin unload \
  --instance actrail.file-leakage
```

卸载会停止新的插件回调，并排空该实例已经提交到告警入口的写请求。卸载不会删除已持久化告警，也不会删除安装目录。

## 故障排查

- `plugin config is required`：加载时缺少 `--plugin-config`，或 Web 发现目录中的配置文件名与 manifest 不匹配。
- `alert-write capability is not granted`：缺少 `--grant alert-write`。
- `trace-file-state-read capability is not granted`：缺少 `--grant trace-file-state-read`。
- `successful file write ... has no complete path`：当前 capture profile 没有提供完整文件路径；检查 `fs-access-basic` 和 daemon 权限。
- `file leakage candidate count ... exceeded`：提高 `candidate_max_count`，或缩小需要观察的工作负载范围。
- 插件已安装但未运行：安装不会自动启用插件；用 Plugins 页面或 `plugin load` 显式加载。
- Web 看不到告警：先检查插件 `status` 的 `last_error`，再检查 daemon 日志和 `[plugins.alerts]` 队列配置。
