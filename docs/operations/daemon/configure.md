# 生成和维护 daemon 配置

> 本文说明如何从内置模板生成可审计的 daemon 配置，并在启动前确认路径、采集与治理边界。

## 生成配置

以下命令假设 release binary 已安装到 `PATH`；从源码 checkout 运行时，可将命令替换为对应的 `./target/release/<binary>`。

系统级配置使用默认路径：

```bash
sudo actraild init
```

本地或独立实例使用显式路径：

```bash
mkdir -p local
actrailctl init --output local/operator.conf
```

若目标文件已存在，`init` 会读取并校验，不会自动覆盖。只有确认旧内容可丢弃时才使用 `--force`。需要在内置默认模板上应用一段 TOML patch 时使用：

```bash
actrailctl init \
  --output local/operator.conf \
  --patch local/operator.patch.toml
```

## 启动前检查

至少审查以下边界：

- `[control]`：socket、PID、日志和并发 trace 数；
- `[storage]` 与 `[storage.retention]`：SQLite 路径和清理周期；
- `[capture]`：该实例声明必须提供的 capabilities；
- `[payload.*]` 与 `[semantic_retention]`：明文采集、容量、redaction 和内容所有权；
- `[enforcement]`、`[command_control]`、`[network_control]`：是否会主动改变工作负载；
- `[export.snapshot]` 与插件 exporter：哪些数据可以离开存储；
- `[supervision]`：启动、停止和轮询时间预算。

同一主机的多个 daemon 实例必须使用不同的 control socket、PID、日志、SQLite、export directory 和 TLS sync socket。配置缺失、值无效或必需 capability 不可用时应修复根因，不得添加静默缩减覆盖范围的 fallback。

字段分组与当前默认值见 [daemon 配置参考](../../reference/configuration/daemon.md) 和 [采集配置参考](../../reference/configuration/collection.md)。
