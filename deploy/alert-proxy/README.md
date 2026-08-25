# actraild-alert-proxy deployment

`actraild-alert-proxy.toml` 是 proxy 进程的完整配置。
`actraild-alert-forwarding.startup.toml` 是需要合并到完整 OperatorConfig 的 daemon 配置片段。
部署前必须替换 `subscriber.allowed_tokens`，并把 daemon ingress 的 UID/GID 调整为实际运行 `actraild` 的身份。
proxy 会拒绝包含部署占位 token 的配置，避免以公开凭据开放 subscriber listener。

`actraild` 的完整 OperatorConfig 使用独立 `[alert_forwarding]` 子配置指向：

- proxy executable；
- 本文件；
- builtin plugin JSON config；
- 与 `daemon_ingress.socket_path` 相同的 UDS path。

对外 TCP listener 默认只监听 loopback。
跨不可信网络部署时必须在 listener 前终止 TLS。
