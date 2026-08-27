# 安全与数据边界

> 本文说明启用 AcTrail 前需要确定的权限、敏感数据、治理和导出边界。

## 权限边界

`actraild` 按启用功能访问以下 Linux 内核接口：eBPF 加载内核观测程序，tracefs 与 uprobe 提供 tracepoint 和用户态函数探针，seccomp 控制 syscall 通知，pidfd 安全引用进程，fanotify 提供文件权限事件。root 身份并不保证容器或宿主策略授予这些能力。必需能力不可用时，启动或使用 `required` 权限策略的 trace 应明确失败；`auto` 只用于明确允许降级的容器权限轴。

Control socket 是管理接口。默认路径 `/run/actrail/control.sock`、默认 mode `0660`；应通过文件属主和组限制可以创建、删除或操作 trace 的主体。Web UI 还可执行本地插件管理操作，因此只应监听可信接口；默认监听 `127.0.0.1:18080`。

## 内容边界

生成的 `default-full-monitor` 配置是覆盖面较广的默认采集 profile，可接触以下敏感信息：

- 进程参数（argv）、环境关联的行为和文件路径；
- stdin、TLS 和普通 socket 中的明文；
- HTTP headers/body、prompt、tool schema、tool result 与模型响应；
- 可被离线猜测的内容 hash。

默认 payload redaction policy 是 `disabled`，并且 snapshot JSON 可包含 payload bytes/text。与此同时，LLM request body 的语义导出默认为 `request_body_export = "none"`。这些是不同层的开关；关闭高层 body export 不代表底层 payload 或 snapshot 一定不含内容。

生产部署应从 [采集配置](../reference/configuration/collection.md) 的各层开关逐一缩小内容面，设置每 trace 容量上限，并保护 SQLite、export directory 和 daemon log。API key 不应放在命令行参数中，因为 argv 本身就是观测对象。

## 治理边界

文件、命令和网络治理会改变工作负载行为。默认生成配置的文件与命令默认决策为 `allow`，但规则与故障决策仍必须按部署策略审查。启用主动治理前应明确：规则作用域、规则文件权限、超时/故障时的决策、审计保留和恢复方式。

## 导出边界

离线 JSON、离线 OpenTelemetry Protocol（OTLP）和实时 exporter 是独立的对外传输面。启用任一出口前，检查目的地、认证、attribute mode 和 body/payload 开关。公开接口默认不应暴露 trace-local block hash 或跨 trace 内容相等性。
