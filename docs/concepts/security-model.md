# 安全与数据边界

> 本文说明启用 AcTrail 前需要确定的权限、敏感数据、治理和导出边界。

## 权限边界

`actraild` 按启用功能访问以下 Linux 内核接口：eBPF 加载内核观测程序，tracefs 与 uprobe 提供 tracepoint 和用户态函数探针，seccomp 控制 syscall 通知，pidfd 安全引用进程，fanotify 提供文件权限事件。root 身份并不保证容器或宿主策略授予这些能力。必需能力不可用时，启动或使用 `required` 权限策略的 trace 应明确失败；`auto` 只用于明确允许降级的容器权限轴。

Control socket 是管理接口。默认路径 `/run/actrail/control.sock`、默认 mode `0660`；应通过文件属主和组限制可以创建、删除或操作 trace 的主体。Web UI 还可执行本地插件管理操作，因此只应监听可信接口；默认监听 `127.0.0.1:18080`。

## 安装与运行文件权限

安装、升级、初始化、启动和更新时，AcTrail 按用途应用下表的权限上限。实际权限取当前权限与上限的按位交集，并清除特殊权限位。已有文件和目录保留属主及属组，新建文件和目录遵守调用者的 umask。

| 产物 | 权限上限 |
| --- | --- |
| RPM 程序、库、插件 WASM、manifest/schema 与专用程序目录 | `0550` |
| RPM 插件配置、主配置、外部规则、插件持久化注册表 | `0640` |
| PID、当前 daemon 日志、主 SQLite 库及其 WAL/SHM/journal | `0640` |
| 专用配置、日志、业务数据及运行目录 | `0750` |
| Control socket、启用 TLS sync 时的 event socket | `0660` |

为自定义文件路径指定 AcTrail 专用父目录。权限上限按文件用途确定。专用父目录和新建的祖先目录权限上限为 `0750`。更上层的既有目录保留其权限，包括共享系统目录、用户主目录和系统临时根目录。

`control.socket_mode_octal` 和 `payload.tls.sync_socket_mode_octal` 接受 `0660` 及更严格的模式，例如 `0600`。配置校验通过后发布 socket。连接权限由 socket 和父目录的属主、属组及权限共同决定。

`actrailctl` 和 Web 在同目录创建临时文件，设置原属主、属组及符合上限的权限，写入完整内容后原子替换主配置。通过符号链接保存时，更新写入链接指向的文件。写入或替换失败时，AcTrail 保留原配置并报告文件路径和原因。读取主配置时仅接受普通文件，遇到 FIFO 等类型会立即返回错误。加载已启用的外部规则和持久化插件注册表时，AcTrail 对文件应用权限上限。配置解析和数据库只读查询使用磁盘上的权限。

以读写模式打开 SQLite 前，AcTrail 对主库和已有的 WAL、SHM、journal 应用权限上限。SQLite 新建附属文件时继承主库权限。为主库设置 `0600` 等与 `0640` 按位相交后仍为非零的权限。相交结果为 `0000` 时，AcTrail 拒绝打开数据库并保留原库。

## 内容边界

生成的 `default-full-monitor` 配置是覆盖面较广的默认采集 profile，可接触以下敏感信息：

- 进程参数（argv）、环境关联的行为和文件路径；
- stdin、TLS 和普通 socket 中的明文；
- HTTP headers/body、prompt、tool schema、tool result 与模型响应；
- 可被离线猜测的内容 hash。

默认 payload redaction policy 是 `disabled`，snapshot JSON 可包含 payload bytes/text。LLM request body 的语义导出默认为 `request_body_export = "none"`。Payload 保留、snapshot 内容和语义正文导出分别由各自的开关控制，部署时须逐层检查。

生产部署应从 [采集配置](../reference/configuration/collection.md) 的各层开关逐一缩小内容面，设置每 trace 容量上限，并保护 SQLite、export directory 和 daemon log。API key 不应放在命令行参数中，因为 argv 本身就是观测对象。

## 治理边界

文件、命令和网络治理会改变工作负载行为。默认生成配置的文件与命令默认决策为 `allow`，但规则与故障决策仍必须按部署策略审查。启用主动治理前应明确：规则作用域、规则文件权限、超时/故障时的决策、审计保留和恢复方式。

## 导出边界

离线 JSON、离线 OpenTelemetry Protocol（OTLP）和实时 exporter 是独立的对外传输面。启用任一出口前，检查目的地、认证、attribute mode 和 body/payload 开关。公开接口默认不应暴露 trace-local block hash 或跨 trace 内容相等性。
