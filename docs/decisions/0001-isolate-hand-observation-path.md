# ADR 0001：隔离手侧观测通路

> 本文记录沙箱观测采用独立 transport、模型、存储和故障域的架构决策。

Status: accepted

## 背景

沙箱 workload 需要资源和 Process I/O 观测，但不能访问可信脑侧 collector、identity graph、semantic pipeline 或主存储。sandbox plugin 或 transport 的下游故障也不能破坏完整 agent 观测。

## 决策

采用独立两跳通路。手侧是运行不可信 workload 的 Guest domain，脑侧是执行完整观测的可信 domain。

```text
actrail-sb -> AF_VSOCK -> actrail-vsock-gateway -> TCP -> actraild
```

`actrail-sb` 独占 Guest eBPF 与 procfs 采集。gateway 是有界 frame proxy，不解释 observation。`GatewayIngestRuntime` 把 raw sandbox observation 互斥路由给匹配插件或独立 Sandbox Evidence DB。

这条通路拥有独立 observation contract、session identity、配置子树、store 和故障域，不复用脑侧 ingest、identity、trace、semantic、recording、export 或主存储。

## 后果

- Guest 失陷只暴露刻意收窄的手侧 observation protocol。
- VMM-specific connection adapter 不进入 observation 或 plugin 语义。
- 运维需要部署并监控 Guest daemon、Host gateway 与 daemon ingest endpoint。
- `(gateway-id, sb-id)` 不隐含跨域 identity 关联；未来关联必须单独评审 contract。
