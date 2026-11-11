# wasm.network-policy-dynamic

该插件通过 WIT `network-control-host` 发布网络规则。Web Configuration 和管理命令操作同一份插件配置，发布失败时保留原有规则。

## 规则

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `decision` | 必填 | allow、deny 或 gray |
| `operation` | connect | connect、send、bind 或 listen |
| `protocol` | any | any、tcp 或 udp |
| `endpoint` | 必填 | 数字 IP 或 CIDR，端口为单值、闭区间或 `*`。例如 `192.0.2.0/24:8000-8100`、`[2001:db8::/48]:443` |
| `rule_id` | 自动生成 | 插件内稳定的规则标识 |

connect/send 匹配远端端点，bind/listen 匹配本地端点。bind 检查请求的端口，零表示申请自动分配。listen 检查 socket 当前绑定的端口。

LSM 支持上述四项操作和协议区分，执行 allow/deny。seccomp 支持 connect/any，包括同步 gray 裁决。seccomp 对其他操作或具体协议的规则在验证和发布时明确报错。

```json
{"rules":[{"rule_id":"deny-tcp-send","decision":"deny","operation":"send","protocol":"tcp","endpoint":"203.0.113.10:*"}]}
```

匹配顺序为更长的 IP 前缀、更窄的端口范围、具体协议优先于 any。同等范围按 owner 和 rule ID 排序。不同操作独立匹配。

同一 owner 可以发布更具体的例外规则。协议和操作相交时，不同 owner 的重叠范围会被拒绝。CIDR 中的主机位会归零。

## 授权

加载时授予规则读取、dry-run、验证及发布能力。发布 grant 同时限制决策、操作、协议和端点范围。例如：

```text
network-policy.rules.apply:kind=deny,operation=send,protocol=tcp,endpoint=203.0.113.10:*
```

grant 的端点可以使用 `*` 覆盖所有地址，规则本身必须提供数字地址或网段。规则的全部地址和端口必须包含在单条 grant 内。操作需要精确匹配，any 协议授权覆盖 tcp/udp，具体协议授权只覆盖自身。Web 加载对话框提供相同的授权字段。

## 管理命令

```text
rule list
rule dry-run <ip:port> [--operation OP] [--protocol PROTOCOL]
rule upsert <allow|deny|gray> <endpoint-scope> [--operation OP] [--protocol PROTOCOL] [--rule-id ID]
rule delete <rule-id>
```

发布 gray 规则还需提供另一个决策插件的 `gray_target`、正数 `timeout_ms`、正数 `concurrency_limit` 和 allow/deny `fallback`。目标不可用时拒绝请求。策略发布插件自身不提供 gray 裁决。
