# wasm.network-policy-dynamic

该 WIT component 是动态网络策略 publisher。它通过 `network-control-host` 把 Web Configuration 或 Plugin command 中的 endpoint/单 IP 全端口规则原子发布给 daemon；allow/deny 在本地热路径完成，gray 才调用另一控制决策插件。本插件自身的 `decide` 始终返回错误，不能作为自己的 `gray_target`。

## 前置条件

daemon 必须启用 `[network_control]` 和 seccomp user-notify。加载时需要授予以下自动权限：

- `network-policy.rules.read`
- `network-policy.rules.match-dry-run`
- `network-policy.rules.validate`
- `network-policy.rules.apply`

`network-policy.rules.apply` 还需要在 Web 加载对话框中选择 allow/deny/gray 和远端范围。grant 范围支持全局 `*`、精确数字 endpoint 和单 IP 全端口 selector：`203.0.113.10:443`、`203.0.113.10:*`、`[2001:db8::10]:443`、`[2001:db8::10]:*`。实际规则不接受裸 `*`。IPv6 scope ID 不参与本地规则匹配。

## 配置

```json
{
  "rules": [
    {
      "rule_id": "deny-example",
      "decision": "deny",
      "remote": "203.0.113.10:*"
    },
    {
      "rule_id": "gray-local",
      "decision": "gray",
      "remote": "127.0.0.1:8443",
      "gray_target": "network-risk-decider",
      "timeout_ms": 500,
      "concurrency_limit": 2,
      "fallback": "deny"
    }
  ]
}
```

配置更新是 AON：插件先读取 revision、校验完整候选，再应用同一 patch。revision 冲突、重复或重叠 selector、grant 越权、非法 selector 或不可用 gray target 会拒绝整批更新，旧配置继续生效。同一 IP 的 `IP:*` 与任何精确端口规则视为重叠，不允许共存。省略 `rule_id` 时插件生成 `network-dynamic-N`。卸载本 publisher 会先撤销它拥有的全部动态规则；gray target 卸载则固定拒绝仍路由到它的待决策连接。

Web 的 Test/Update 与 Plugin command 操作同一份插件内存配置。`--persist` 只持久化插件加载记录，不会把之后的 Web Configuration 更新写入磁盘；需要重启后仍生效的安全基线应放在 `network_control.rules_path` 或初始插件配置中。

## 管理命令

```text
help
rule list
rule dry-run <ip:port>
rule upsert <allow|deny|gray> <ip:port|ip:*> [--rule-id ID]
  [--gray-target INSTANCE --timeout-ms N --concurrency N --fallback allow|deny]
rule delete <rule-id>
```

## 控制边界

当前只治理 `AF_INET`/`AF_INET6` 的 `connect(2)`，不是完整 egress firewall，也不假设 fd 是 TCP socket。端口只支持精确值或单 IP 的全部端口 `*`，不支持任意端口区间。域名、CIDR、DNS、TLS SNI、代理最终目标、`sendto(2)`、AF_UNIX、继承连接和非 `SYS_connect` I/O 均不在 v1 范围内。
