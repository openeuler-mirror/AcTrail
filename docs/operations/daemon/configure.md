# 生成和维护 daemon 配置

> 本文说明如何从内置模板生成可审计的 daemon 配置，并在启动前确认路径、采集与治理边界。

## 生成配置

以下命令假设 release binary 已安装到 `PATH`。从源码 checkout 运行时，可将命令替换为对应的 `./target/release/<binary>`。

系统级配置使用默认路径：

```bash
sudo actrailctl init
```

本地或独立实例使用显式路径：

```bash
mkdir -p local
actrailctl init --output local/operator.conf
```

`--mode` 选择初始化预设，名称不区分大小写：

| 模式 | 别名 | 配置内容 |
| --- | --- | --- |
| `complete`（默认） | `C`、`c` | TLS/socket 使用 `bpf-copy`，seccomp-notify 关闭；按默认规则保留 LLM 内容和工具语义，保留默认 LSM 治理设置 |
| `profile` | `P`、`p` | 保留 LLM 身份、关联、时序和结束状态；关闭请求/响应正文、工具动作记录、用量统计、执行轨迹、MCP、文件读写统计及独立 SSE/HTTP2 明细 |

Profile 模式的具体设置：

- **采集与治理**：使用 TLS/socket `bpf-copy`，关闭 seccomp-notify、文件/命令/网络治理及相应治理 capability。
- **文件观察**：文件路径变更和可写打开观察仍启用。
- **TLS 发现**：动态 TLS 发现沿用默认设置。
- **采集上限**：TLS 单段和单次操作捕获上限均为 65535 字节，限采仍按实际完整性记录。
- **存储与路径**：存储后端和运行路径沿用产品默认。

```bash
actrailctl init --mode P --output local/profile.conf
actrailctl init --mode complete --output local/complete.conf
```

- **生效时间**：模式在生成配置时生效。
- **默认值**：`complete` 沿用产品默认值。
- **覆盖顺序**：配置生成顺序为默认配置、模式预设、命令行中依次指定的 `--patch`。表中的字段递归合并，同名值由后面的 patch 覆盖。
- **配置校验**：全部 patch 合并后校验最终配置，再写入目标文件。
- **seccomp 后端**：需要 seccomp 补采或治理后端时，在 patch 中显式开启 `[seccomp_notify] enabled = true` 并选择对应后端。

若目标文件已存在，未指定模式或 patch 的 `init` 只读取并校验，不会自动覆盖。对已有文件显式指定 `--mode` 或 `--patch` 时必须同时使用 `--force`，以重建配置。按顺序应用运行路径和测例配置时使用：

```bash
actrailctl init \
  --mode profile \
  --output local/operator.conf \
  --patch local/runtime.patch.toml \
  --patch local/patch_config.toml
```

## 启动前检查

至少审查以下边界：

- `[control]`：socket、PID、日志和并发 trace 数。
- `[storage]` 与 `[storage.retention]`：观测存储后端、SQLite 路径和清理周期。
- `[capture]`：该实例必须提供的采集能力。
- `[payload.*]` 与 `[semantic_retention]`：明文采集、容量、脱敏和各层内容保留规则。
- `[enforcement]`、`[command_control]`、`[network_control]`：是否会主动改变工作负载。
- `[export.snapshot]` 与插件 exporter：哪些数据可以离开存储。
- `[supervision]`：启动、停止和轮询时间预算。

- **实例隔离**：同一主机的多个 daemon 实例必须使用不同的 control socket、PID、日志、SQLite、导出目录和 TLS sync socket。
- **配置校验**：启动前补齐所需配置、修正无效值，并确认必需采集能力可用。

配置项的完整路径、类型、默认值和必填条件见[配置布局](../../architecture/config-layout.md)。表中默认值表示省略字段时采用的模型默认值；`profile` 模式和用户 patch 按上面的顺序覆盖这些值。字段的使用方式见 [daemon 配置参考](../../reference/configuration/daemon.md) 和 [采集配置参考](../../reference/configuration/collection.md)。

## 仅在线处理

默认观测存储为 SQLite。需要丢弃观测写入时，在 patch 中设置：

```toml
[storage]
backend = "noop"
```

- **数据库**：NoOp 不创建主 SQLite 数据库。
- **在线处理**：在线采集、分析和已配置的在线输出继续运行。
- **历史查询**：历史列表为空，显式 payload 读取返回 NotFound，依赖历史的快照与离线查询不能提供已丢弃的数据。

`sandbox_alerts.enabled` 和 `hand_observation.enabled` 默认均为 `false`。显式启用这两个独立功能时，其 `sandbox-alerts.sqlite`、`sandbox-evidence.sqlite` 仍按各自配置创建。
