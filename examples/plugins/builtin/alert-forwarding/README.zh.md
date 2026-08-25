# Alert Forwarding builtin plugin

该 builtin plugin 控制 `actraild` 是否把已成功写入主告警存储的告警发送给 `actraild-alert-proxy`。

`categories` 与 `AlertDefinition.kind` 精确匹配。
`all_categories=true` 时 `categories` 必须为空。
只有 daemon 与 proxy 完成连接握手后，`enabled=true` 才会生效。
