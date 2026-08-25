# Sandbox Resource Alert 内置插件包

类别：内置手侧观测消费者。

该插件消费独立 Hand 通路中的 `process-io`、`guest-resource` 和 `oom-victim` observation，生成以下
本地告警记录：

- Guest 内核报告 OOM victim，并携带 victim PID、命令名和被观测谱系归因；
- Guest 可用内存低于配置阈值；
- Guest 区间 CPU 利用率越过配置阈值；
- 一个采样区间内进程谱系读取或写入字节数超过配置阈值。

manifest 的 `role.sandbox-observation-consumer.subscriptions.observation_kinds` 接受任意
非空子集。默认 manifest 同时订阅三类 observation；只需 I/O 告警的实例可以仅配置
`process-io`，OOM kill 需要订阅 `oom-victim`，OOM risk 需要订阅 `guest-resource`。
未被任何已加载实例订阅的 observation 会进入独立 NoInterest evidence
数据库，不会回退给其他插件。

例如，仅订阅 Guest 资源 observation：

```toml
[role.sandbox-observation-consumer.subscriptions]
observation_kinds = ["guest-resource"]
```

插件不进入脑侧 Ingest、Identity、Trace、Semantic、Recording、Export 或主 Storage。
告警通过独立有界队列写入 operator 配置的 Sandbox Alert DB。
数据库事务提交成功后，才尝试交给 builtin forwarding plugin 外发。
数据库或外发支路故障不反向影响 Hand observation ingestion。

包内文件：

- `sandbox-resource-alert.plugin.toml`：可由 `actraild` 加载的 builtin manifest；
- `sandbox-resource-alert.config.json`：默认 JSON 配置；
- `sandbox-resource-alert.config.v1.schema.json`：配置 JSON Schema。

默认阈值按每个 `actrail-sb` 采样区间计算：CPU 90%，可用内存 512 MiB，读取和写入各
256 MiB。部署方应结合 SB 的采样周期和 Guest 规格调整阈值。

## 源码树内加载

下面的路径必须替换为当前 checkout 的绝对路径。`actraild` 不负责按 operator 配置文件
所在目录解析相对路径。

```toml
[plugins.startup]
enabled = true
failure_policy = "fail-fast"

[[plugins.startup.load]]
instance = "sandbox-resource-alert.default"
enabled = true
manifest = "/absolute/path/to/AcTrail/examples/plugins/builtin/sandbox-resource-alert/sandbox-resource-alert.plugin.toml"
plugin_config = "/absolute/path/to/AcTrail/examples/plugins/builtin/sandbox-resource-alert/sandbox-resource-alert.config.json"
host_grants = []
```

## 部署路径

生产部署建议保持 manifest/schema 与可修改配置分离：

```text
/usr/share/actrail/plugins/sandbox-resource-alert/
├── sandbox-resource-alert.plugin.toml
└── sandbox-resource-alert.config.v1.schema.json

/etc/actrail/plugins/sandbox-resource-alert/
└── sandbox-resource-alert.config.json

/var/lib/actrail/
└── sandbox-alerts.sqlite
```

对应的 startup 配置为：

```toml
[plugins.startup]
enabled = true
failure_policy = "fail-fast"

[[plugins.startup.load]]
instance = "sandbox-resource-alert.default"
enabled = true
manifest = "/usr/share/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.plugin.toml"
plugin_config = "/etc/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.config.json"
host_grants = []
```

该 builtin 不接受 host grants。manifest 或配置缺失、字段未知、阈值非法、订阅集合为空
或含重复项时应让 daemon 启动失败，不应回退到主 Storage 或其他插件。
