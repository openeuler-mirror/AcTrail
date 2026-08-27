# ADR 0004：使用独立告警代理

> 本文记录将 subscriber session 置于 `actraild` 之外的架构决策及其可靠性成本。

Status: accepted

## 背景

alert producer 需要低开销、非阻塞地向多个已鉴权 subscriber 发送数据。subscriber 的慢速、serialization 与 reconnect 不能进入告警评估或持久化通路。

## 决策

daemon 侧 admission 保留在 builtin forwarding plugin，把外部 session 移到独立 `actraild-alert-proxy`。daemon 与 proxy 使用一条持久、已鉴权的 AF_UNIX stream（本地 Unix-domain socket）和有界 binary frame。proxy 拥有一个有界 broadcast queue，每个 subscriber 拥有独立有界 queue 与 writer。

daemon 使用 immutable enablement/category snapshot 与 connection generation。proxy 对每条外部 alert 只编码一次，并在匹配 subscriber 间共享 immutable bytes。

## 后果

- producer 热路径只读取配置并执行 nonblocking queue admission。
- proxy 或 subscriber 失败不能阻塞或回滚 alert persistence。
- disable 或 reconnect 时丢弃旧 generation record。
- 部署增加一个独立进程、鉴权边界、socket 和运维 health surface。
