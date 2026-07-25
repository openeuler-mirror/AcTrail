# 请求/响应增长与长命令告警插件使用手册

本文说明 `actrail.activity-anomaly` 插件的阈值配置、加载更新、手动验证和告警查询方法。

## 1. 准备路径变量

根据实际部署位置设置以下变量：

```bash
export ACTRAIL_REPO="<path-to-AcTrail>"
export ACTRAIL_BIN_DIR="$ACTRAIL_REPO/target/release"
export ACTRAIL_OPERATOR_CONFIG="<path-to-operator.conf>"
export ACTRAIL_PLUGIN_DIR="<path-to-activity-anomaly-plugin-directory>"
```

尖括号中的内容必须替换为实际路径。daemon、Web 和 CLI 必须使用同一个 `ACTRAIL_OPERATOR_CONFIG`。

## 2. 配置告警阈值

阈值配置文件为：

```text
$ACTRAIL_PLUGIN_DIR/activity-anomaly.config.json
```

默认规则如下：

| 告警类型 | 默认规则 |
| --- | --- |
| 请求增长 | 请求达到 2 MiB，或达到同组历史中位数的 2 倍并满足最小样本和增量要求 |
| 响应增长 | 响应达到 4 MiB，或达到同组历史中位数的 3 倍并满足最小样本和增量要求 |
| 长命令 | Agent 顶层命令耗时超过 60 秒 |

主要配置项：

| 配置项 | 说明 |
| --- | --- |
| `enabled` | 是否启用该规则 |
| `hard_limit_bytes` | 请求或响应的固定字节阈值 |
| `window_size` | 历史样本窗口大小 |
| `minimum_samples` | 执行相对增长判断所需的最少样本数 |
| `ratio_per_mille` | 相对增长倍数，`2000` 表示 2 倍 |
| `minimum_growth_bytes` | 相对历史中位数的最小增量 |
| `minimum_current_bytes` | 参与相对增长判断的最小当前值 |
| `maximum_duration_ms` | 命令时长阈值，单位为毫秒 |

插件仅在加载时读取配置。修改配置后，需要卸载并重新加载插件。

## 3. 启动服务

查询 daemon 状态：

```bash
sudo "$ACTRAIL_BIN_DIR/actraild" \
  --config "$ACTRAIL_OPERATOR_CONFIG" \
  status
```

daemon 未运行时执行：

```bash
sudo "$ACTRAIL_BIN_DIR/actraild" \
  --config "$ACTRAIL_OPERATOR_CONFIG" \
  start
```

启动 Web：

```bash
sudo "$ACTRAIL_BIN_DIR/actrailweb" \
  --config "$ACTRAIL_OPERATOR_CONFIG" \
  --addr 127.0.0.1 \
  --port 18080
```

Web 地址为 `http://127.0.0.1:18080`。

## 4. 管理插件

### 4.1 加载

```bash
sudo "$ACTRAIL_BIN_DIR/actraild" \
  --config "$ACTRAIL_OPERATOR_CONFIG" \
  plugin load \
  --manifest "$ACTRAIL_PLUGIN_DIR/activity-anomaly.plugin.toml" \
  --plugin-config "$ACTRAIL_PLUGIN_DIR/activity-anomaly.config.json" \
  --grant trace-activity-read \
  --grant alert-write \
  --instance actrail.activity-anomaly \
  --persist
```

参数说明：

| 参数 | 说明 |
| --- | --- |
| `--config` | daemon 的 operator 配置文件 |
| `plugin load` | 加载插件 |
| `--manifest` | 插件清单文件 |
| `--plugin-config` | 插件阈值配置文件 |
| `--grant trace-activity-read` | 允许插件读取活动事实 |
| `--grant alert-write` | 允许插件写入告警 |
| `--instance` | 插件实例名称 |
| `--persist` | 保存注册信息，使 daemon 重启后自动恢复插件 |

查询插件状态：

```bash
sudo "$ACTRAIL_BIN_DIR/actraild" \
  --config "$ACTRAIL_OPERATOR_CONFIG" \
  plugin status \
  --instance actrail.activity-anomaly
```

正常状态应包含：

```text
state=active
last_error=none
```

### 4.2 更新

更新阈值、WASM 或插件清单时：

1. 等待当前被观测的 Agent trace 结束。
2. 修改配置或替换插件文件。
3. 卸载插件：

   ```bash
   sudo "$ACTRAIL_BIN_DIR/actraild" \
     --config "$ACTRAIL_OPERATOR_CONFIG" \
     plugin unload \
     --instance actrail.activity-anomaly \
     --persist
   ```

4. 重新执行第 4.1 节的加载命令。
5. 执行 `plugin status` 确认插件状态。

该更新过程不需要重启 daemon，也不需要重启通过 `actrailctl launch` 启动的 Agent。卸载期间未结束的 trace 不会由新插件实例补充分析。

### 4.3 卸载

```bash
sudo "$ACTRAIL_BIN_DIR/actraild" \
  --config "$ACTRAIL_OPERATOR_CONFIG" \
  plugin unload \
  --instance actrail.activity-anomaly \
  --persist
```

`--persist` 会同时删除持久化注册信息。未使用持久化加载时，卸载命令也应省略 `--persist`。

## 5. 手动验证

本节仅用于开发环境。验证前，将配置文件中的以下阈值临时调低：

```json
{
  "request_growth": {
    "hard_limit_bytes": 1
  },
  "response_growth": {
    "hard_limit_bytes": 1
  },
  "command_duration": {
    "maximum_duration_ms": 500
  }
}
```

以上内容仅表示需要修改的字段，不是完整配置文件。其他原有字段必须保留。

备份并编辑配置：

```bash
sudo cp -- \
  "$ACTRAIL_PLUGIN_DIR/activity-anomaly.config.json" \
  "$ACTRAIL_PLUGIN_DIR/activity-anomaly.config.json.before-manual-test"
sudoedit "$ACTRAIL_PLUGIN_DIR/activity-anomaly.config.json"
```

按照第 4.2 节重新加载插件，然后设置真实 xiaoO 的路径：

```bash
export ACTRAIL_XIAOO_BIN="<path-to-xiaoo>"
export ACTRAIL_XIAOO_CONFIG="<path-to-xiaoo-config>"
```

运行真实 Agent：

```bash
sudo -E "$ACTRAIL_BIN_DIR/actrailctl" \
  --config "$ACTRAIL_OPERATOR_CONFIG" \
  launch \
  --name manual-activity-anomaly \
  --host-ebpf required \
  --seccomp-notify required \
  -- \
  "$ACTRAIL_XIAOO_BIN" \
  --cli run \
  --config "$ACTRAIL_XIAOO_CONFIG" \
  --tools bash \
  --max-turns 3 \
  --prompt '请使用 bash 工具执行 sleep 2，然后结束任务。'
```

xiaoO 必须已配置可用的真实 LLM provider。Agent 结束后，在 Web 的“告警”页面确认以下告警：

| 告警类型 | 预期结果 |
| --- | --- |
| `llm.request.growth` | 显示请求大小和 1 字节阈值 |
| `llm.response.growth` | 显示响应大小和 1 字节阈值 |
| `command.duration.exceeded` | 显示 `sleep 2`、实际耗时和 500 ms 阈值 |

验证完成后恢复生产配置，并按照第 4.2 节重新加载插件：

```bash
sudo mv -- \
  "$ACTRAIL_PLUGIN_DIR/activity-anomaly.config.json.before-manual-test" \
  "$ACTRAIL_PLUGIN_DIR/activity-anomaly.config.json"
```

## 6. 使用说明

- 告警在 Agent trace 结束后生成，不会在命令运行过程中立即产生。
- 完整命令行依赖 seccomp notify；未采集 argv 时只能显示可执行文件。
- 请求和响应的历史窗口按 trace、进程、模型、服务端和 URL 隔离。
- 多容器场景中，各容器不会共享增长检测窗口。
- 配置字段缺失或取值无效会导致插件加载失败，具体原因可通过 `plugin status` 的 `last_error` 查看。
- 通过 `[plugins.startup]` 管理插件时，配置更新需要重启 daemon，不应与 `--persist` 同时使用。
