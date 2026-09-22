# 生成和维护 daemon 配置

> 本文说明如何从内置模板生成可审计的 daemon 配置，并在启动前确认路径、采集与治理边界。

## 生成配置

以下命令假设 release binary 已安装到 `PATH`；从源码 checkout 运行时，可将命令替换为对应的 `./target/release/<binary>`。

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
| `complete`（默认） | `C`、`c` | 当前产品默认配置；按默认规则保留 LLM 内容和工具语义，保留默认治理设置 |
| `profile` | `P`、`p` | 保留 LLM 身份、关联、时序和终态；关闭请求/响应正文、工具投影、usage、trajectory、MCP、文件读写统计及独立 SSE/HTTP2 明细 |

Profile 使用 TLS/socket `bpf-copy`，关闭 seccomp-notify、文件/命令/网络治理及相应治理 capability。文件路径变更和可写打开观察仍启用；动态 TLS 发现沿用默认设置。TLS 单段和单次操作捕获上限均为 65535 字节，限采仍按实际完整性记录。存储后端和运行路径沿用产品默认。

```bash
actrailctl init --mode P --output local/profile.conf
actrailctl init --mode complete --output local/complete.conf
```

模式仅用于生成配置，不是 daemon 的运行时开关。`complete` 表示产品默认值，不会强制开启默认关闭的采集或治理功能。配置生成顺序为默认配置、模式预设、用户 `--patch`，因此 patch 可以覆盖模式中的设置；最终配置仍必须通过校验。

若目标文件已存在，未指定模式或 patch 的 `init` 只读取并校验，不会自动覆盖。对已有文件显式指定 `--mode` 或 `--patch` 时必须同时使用 `--force`，以重建配置。需要在预设上应用一段 TOML patch 时使用：

```bash
actrailctl init \
  --mode profile \
  --output local/operator.conf \
  --patch local/operator.patch.toml
```

## 启动前检查

至少审查以下边界：

- `[control]`：socket、PID、日志和并发 trace 数；
- `[storage]` 与 `[storage.retention]`：观测存储后端、SQLite 路径和清理周期；
- `[capture]`：该实例声明必须提供的 capabilities；
- `[payload.*]` 与 `[semantic_retention]`：明文采集、容量、redaction 和内容所有权；
- `[enforcement]`、`[command_control]`、`[network_control]`：是否会主动改变工作负载；
- `[export.snapshot]` 与插件 exporter：哪些数据可以离开存储；
- `[supervision]`：启动、停止和轮询时间预算。

同一主机的多个 daemon 实例必须使用不同的 control socket、PID、日志、SQLite、export directory 和 TLS sync socket。配置缺失、值无效或必需 capability 不可用时应修复根因，不得添加静默缩减覆盖范围的 fallback。

字段分组与当前默认值见 [daemon 配置参考](../../reference/configuration/daemon.md) 和 [采集配置参考](../../reference/configuration/collection.md)。

## 仅在线处理

默认观测存储为 SQLite。需要丢弃观测写入时，在 patch 中设置：

```toml
[storage]
backend = "noop"
```

NoOp 不创建主 SQLite 数据库；在线采集、分析和已配置的在线输出继续运行。历史列表为空，显式 payload 读取返回 NotFound，依赖历史的快照与离线查询不能提供已丢弃的数据。

`sandbox_alerts.enabled` 和 `hand_observation.enabled` 默认均为 `false`。显式启用这两个独立功能时，其 `sandbox-alerts.sqlite`、`sandbox-evidence.sqlite` 仍按各自配置创建。
