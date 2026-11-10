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

`init` 默认导出所属模块提供的完整配置，导出筛选默认关闭。策略文件存在时，仍须显式传入 `--filter` 才会筛选：

```bash
actrailctl init --filter --output local/delivery.conf
```

启用筛选后，生成文件的默认配置项由 [`crates/core/config_export_filter/allowlist.toml`](../../../crates/core/config_export_filter/allowlist.toml) 选择。Web 配置编辑器使用同一份策略决定展示字段。配置字段的类型、默认值和校验由各配置模块提供。初始模式为白名单，包含：

| 配置项 | 配置路径 |
| --- | --- |
| 控制套接字 | `control.socket_path` |
| PID 文件 | `control.pid_file` |
| 日志文件 | `control.log_path` |
| 诊断日志级别 | `control.diagnostic_log_level` |
| 插件发现目录列表 | `plugins.discovery.directory` |
| SQLite 数据库位置 | `storage.sqlite.path` |
| SQLite 繁忙等待超时 | `storage.sqlite.busy_timeout_ms` |
| Web 监听地址 | `web.listen_addr` |
| TLS 同步事件套接字 | `payload.tls.sync_event_socket_path` |
| TLS 脱敏策略 | `payload.tls.redaction_policy` |
| 标准输入输出脱敏策略 | `payload.stdio.redaction_policy` |
| 套接字脱敏策略 | `payload.socket.redaction_policy` |

策略通过编译嵌入二进制，调整交付范围时修改该文件并重新构建。`default = "whitelist"` 选择 `whitelist` 字符串数组中的路径；`default = "blacklist"` 选择完整默认文档中排除 `blacklist` 后的字段。黑名单初始为空。两份列表独立生效，均会校验；字段可同时出现在两个列表中。`default` 仅选择名单模式，导出筛选仍须显式启用。

名单选择叶子字段或整个数组，路径必须对应默认模板中的有效值。源码中的策略文件缺失时仍可构建并使用完整配置导出；`init --filter` 和 Web 编辑器初始化需要策略文件，缺失、格式错误、同一列表内重复路径、未知配置项或非法模式会产生明确错误。普通完整导出直接使用所属模块模板，策略文件缺失或无效时均可使用。

配置文件可手工添加隐藏字段。加载配置时，省略字段采用所属模块默认值；手写字段按原有规则校验并生效。Web 编辑器按策略展示字段，保存可见字段时保留文件中已有隐藏字段及其值。复杂数组使用 TOML 文本控件，填写单个 TOML 值，保存时由所属模块校验。

Web 只读启动快照展示启动时读取的配置文件原文。普通 `init` 生成的文件展示完整默认配置项，`init --filter` 生成的文件展示筛选后的实际配置项，手写隐藏项也随文件内容展示。Web 命令行覆盖作用于运行配置，摘要根据启动时的生效配置生成。保存配置会更新文件，快照和摘要在重启 Web 后重新生成。

`--mode` 选择初始化预设，名称不区分大小写：

| 模式 | 别名 | 配置内容 |
| --- | --- | --- |
| `complete`（默认） | `C`、`c` | TLS/socket 使用 `bpf-copy`，seccomp-notify 关闭；按默认规则保留 LLM 内容和工具语义，保留默认 LSM 治理设置 |
| `profile` | `P`、`p` | 保留 LLM 身份、关联、时序和结束状态；关闭请求/响应正文、工具动作记录、用量统计、执行轨迹、MCP、文件读写统计及独立 SSE/HTTP2 明细 |

Profile 模式的具体设置：

- **采集与治理**：使用 TLS/socket `bpf-copy`，关闭 seccomp-notify、文件/命令/网络治理及相应治理 capability。
- **文件观察**：文件路径变更和可写打开观察仍启用。
- **TLS 发现**：动态 TLS 发现沿用默认设置。
- **上下文压缩**：关闭依赖轨迹和正文的通用压缩推断。
- **采集上限**：TLS 单段和单次操作捕获上限均为 65535 字节，限采仍按实际完整性记录。
- **存储与路径**：存储后端和运行路径沿用产品默认。

```bash
actrailctl init --mode P --output local/profile.conf
actrailctl init --mode complete --output local/complete.conf
```

- **生效时间**：模式在生成配置时生效。
- **默认值**：`complete` 沿用产品默认值。
- **覆盖顺序**：默认从完整配置开始；传入 `--filter` 时先按策略选择默认项。随后合并显式 `--mode` 预设和命令行中依次指定的 `--patch`。表中的字段递归合并，数组整体替换，同名值由后面的 patch 覆盖。
- **显式字段**：完整导出和筛选导出均保留预设和 patch 声明的字段和值，包括名单隐藏的字段及与默认值相同的显式值。黑名单不禁止预设或 patch 显式写入字段。Profile 预设同时写入治理 capability 裁剪后的数组，保证重载后保留该模式的采集与治理设置。Web 展示继续使用选择策略。
- **配置校验**：全部 patch 合并后校验最终配置，再写入目标文件。
- **文件写入**：在目标同目录的临时文件中完成写入后提交；新建时不覆盖已占用的路径，`--force` 原子替换普通文件并保留权限位及所属用户、组。符号链接保留，更新其实际目标；设备节点、FIFO 等特殊文件拒绝覆盖。提交前发生写入或权限设置错误时，原文件保持完整，临时文件清理。该操作要求目标父目录可写；新文件权限遵循进程 umask。
- **seccomp 后端**：需要 seccomp 补采或治理后端时，在 patch 中显式开启 `[seccomp_notify] enabled = true` 并选择对应后端。

若目标文件已存在，未指定筛选、模式或 patch 的 `init` 只读取并校验。对已有文件指定 `--filter`、`--mode` 或 `--patch` 时必须同时使用 `--force`，以重建配置。按顺序应用运行路径和测例配置时使用：

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

## 网络拒绝告警

seccomp 网络规则拒绝在首次收到时生成告警，后续拒绝按
`network_control.alert_window_ms` 汇总，默认 60000 毫秒；trace 结束和正常停机也会提交
剩余增量。`network_control.alert_max_groups` 默认 256，限制每窗口跨 trace 新准入的
规则、revision 和决策来源分组；trace 结束后准入额度保留至窗口结束。每组最多产生首次
和增量两条告警，每窗口最多尝试提交两倍分组上限的告警，超出分组容量的拒绝数量会记录
汇总日志。载荷中的端点、进程和原因取自该增量的首条拒绝；`received_denials` 表示收到的
拒绝数。两项配置均须大于 0。

这两项配置只控制 seccomp 网络告警聚合，独立于 `access_control.backends.lsm` 聚合参数和
`network_control.audit_enabled`。关闭审计仍会生成规则拒绝告警。默认裁决、故障回退和
上下文捕获失败继续使用各自的审计或错误日志表达。

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
