# AcTrail host OTel Collector（验收/开发环境）

本目录提供一个边界明确的主机侧 OpenTelemetry Collector 部署，用于接收 AcTrail 虚拟容器 Guest 通过 OTLP/HTTP 导出的 traces。Compose 固定使用官方 `otel/opentelemetry-collector-contrib:0.157.0` 镜像，并启用主机网络、只读根文件系统、非 root 用户、删除全部 capabilities 和 `no-new-privileges`。容器同时设置 384 MiB cgroup 上限、`GOMEMLIMIT=320MiB` Go runtime soft limit、128 PID 上限和有界 Docker JSON 日志轮转。

这不是生产级遥测后端。默认接收端是明文 HTTP，没有 TLS、认证或租户隔离；`debug` exporter 还可能把 span 属性中的敏感数据写入容器日志。只应在受控的开发/验收网络中使用，并通过主机防火墙限制 `4318/tcp` 的来源。生产环境应另行配置 TLS/mTLS、认证、访问控制和真正的持久化后端。

## Guest 地址必须指向主机

Guest 中的 AcTrail OTLP/HTTP endpoint 应类似：

```text
http://<GUEST_可达的主机_IP>:4318/v1/traces
```

不要在 Guest 中配置 `http://127.0.0.1:4318/v1/traces`：Guest 里的 `127.0.0.1` 指向 Guest 自己，不是宿主机。请先确认 Kata/虚拟机网络路由和主机防火墙允许 Guest 访问所选主机地址。

生成 Guest 镜像时，把这个完整 URL 通过 `guest/inject-image.sh --otel-endpoint` 显式
注入；安装器不会把 bundle 中的 `COLLECTOR_HOST` 占位值带入镜像。

`.env` 中的 `OTELCOL_OTLP_HTTP_ENDPOINT` 是 Collector 的主机监听地址，格式为 `IP:端口`，不是带 scheme 或 `/v1/traces` 的 URL。示例使用 `0.0.0.0:4318` 只是为了开发环境中便于 Guest 连接；若网络模型允许，优先绑定专用的 Guest 可达主机地址。

## `host.id` 的边界

当前 Collector 不依赖 `host.id` 做接入、路由或落盘，也不会在接收路径同步查询它；
它只原样接收 Guest OTLP resource 中可选的 `host.id`。AcTrail daemon 在 attach 时已经
把当时解析出的值放入 trace 快照，后续 control/Web 查询和导出都只读该快照。

在 Kata 拓扑里，标准 `host.id` 表示运行 daemon 的 Guest OS/microVM 实例，不表示承载
Collector 的物理节点。若以后需要跨物理节点汇聚，应另设语义独立的 Collector/node
归属（例如自定义 `actrail.collector.host.id` 或权威基础设施 resource enrichment），
不能覆盖 Guest 的 `host.id`。本验收配置暂不制造这个字段。

## 启动

```bash
cd deploy/virtual-container/host-collector
cp .env.example .env
# 按实际网络和数据目录修改 .env
sudo install -d -o 10001 -g 10001 -m 0750 /var/lib/actrail/otelcol
docker compose --env-file .env config
docker compose --env-file .env up -d
docker compose --env-file .env ps
```

如果修改了 `OTELCOL_DATA_DIR`，上面的 `install` 路径也必须同步修改。该 bind mount 是容器唯一的持久可写数据目录；根文件系统保持只读，临时目录使用受限 tmpfs。
Compose 对配置使用共享 SELinux relabel（`z`），对 Collector 数据目录使用私有
relabel（`Z`）；不启用 SELinux 的平台会忽略这两个选项。两个 bind source 都要求
预先存在，Compose 不会静默创建路径。

## 健康、日志和验收数据

健康检查扩展只监听宿主机 loopback，因此应在宿主机执行：

```bash
curl --fail --show-error http://127.0.0.1:13133/
```

查看 Collector 日志和 `debug` exporter 输出：

```bash
docker compose --env-file .env logs --tail=100 otelcol-contrib
```

Guest 发送 trace 后，确认轮转 JSON 文件已经产生并包含 span。下面使用 `.env.example` 的默认目录；自定义目录时请替换路径：

```bash
sudo find /var/lib/actrail/otelcol -maxdepth 1 -type f -name 'actrail-traces.json*' -ls
sudo tail -n 5 /var/lib/actrail/otelcol/actrail-traces.json
```

停止并删除容器（保留主机数据目录）：

```bash
docker compose --env-file .env down
```

## 可靠性边界

`memory_limiter` 和 `batch` 只提供进程内的限流与批处理；Compose 的 cgroup 上限才是最后的硬边界，极端压力下 Collector 仍可能被 OOM kill。轮转 JSON file exporter 是验收证据和调试产物，不是 WAL，也不提供 exactly-once 或 at-least-once 保证。Guest 侧导出、Collector 进程或宿主机异常退出时，尚未成功写出的 span 可能丢失；需要故障恢复语义时，应增加具备持久队列/WAL 的链路和持久化遥测后端，并单独验证其恢复行为。

当前选择 bounded rotation，因此没有启用与 rotation 不兼容的 `append`。file exporter
默认以 truncate 方式打开当前 active 文件；Collector 容器重启时，
`actrail-traces.json` 可能被截断，已轮转的备份不等于可靠恢复日志。需要保留一次验收
证据时，应在重启 Collector 前先把整个数据目录归档；生产留存不要依赖该文件。
