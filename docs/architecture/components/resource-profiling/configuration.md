# 资源观测配置

资源观测配置位于 operator 配置的 `resource_profiling` 表中。它控制主机和工作负载的采集、样本存储与资源告警。各字段的完整路径、默认值和必填条件见[配置布局](../../config-layout.md)，实现位置见[代码布局](../../code-layouts/crates/core/resource_profiling/code-layout.md)。

## 配置所有权

`collection` 负责采集方式和周期。`host` 采集宿主机的 CPU、内存和负载；`workload` 采集被观测进程及其所属资源分组。`procfs` 后端读取 Linux 的 `/proc` 进程信息，`cgroup` 后端读取内核资源分组的计数。`auto` 模式根据可用能力选择后端，配置错误会终止对应配置的加载。

`storage` 负责样本交付与保留。`database` 后端写入本地数据库，`remote` 后端发送到外部指标服务，`hybrid` 后端将同一批样本分发到多个目标。每个后端解析自己的子配置。各目标独立处理写入失败、超时和队列满，并记录交付结果。

Viewer 和 Web 通过本地数据库查询资源样本。使用 hybrid 时，这些查询要求恰好一个 `database` 子后端。只有远端目标时，通过相应的接收端查询数据。

`alerts` 负责工作负载资源告警阈值。CPU、进程常驻内存和 cgroup 内存阈值分别配置；阈值为 `disabled` 时关闭对应告警。采集开关由 `collection` 管理。

## 同时写入本地数据库和远端服务

下面的配置为本地 SQLite 和 Prometheus 接收端各定义一个目标。使用时，将远端地址替换为实际接收端地址，并按需填写该目标的 `headers` 请求头。

```toml
[resource_profiling.storage]
backend = "hybrid"

[resource_profiling.storage.hybrid]
queue_capacity_batches = 64
write_timeout_ms = 1000

[[resource_profiling.storage.hybrid.backends]]
name = "local"
backend = "database"
[resource_profiling.storage.hybrid.backends.database]
backend = "sqlite"
[resource_profiling.storage.hybrid.backends.database.sqlite]
path = "./actrail-resources.sqlite"

[[resource_profiling.storage.hybrid.backends]]
name = "metrics"
backend = "remote"
[resource_profiling.storage.hybrid.backends.remote]
protocol = "prometheus_remote_write"
endpoint = "http://127.0.0.1:9090/api/v1/write"
timeout_ms = 1000
```
