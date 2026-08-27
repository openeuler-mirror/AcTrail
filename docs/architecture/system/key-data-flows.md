# 关键数据流

> 本文展示观测与告警从生产者到允许使用方的数据流及其故障边界。

下图分开表示三条数据流。实线箭头表示投递步骤；每一行的最终节点是该类数据不会自动越过的边界。

```mermaid
flowchart LR
  subgraph Brain[Brain-side 观测]
    B1[内核、seccomp 或 TLS 明文] --> B2[摄入]
    B2 --> B3[身份和 trace 关联]
    B3 --> B4[协议和语义投影]
    B4 --> B5[主存储、导出器和治理]
  end

  subgraph Hand[Hand-side 观测]
    H1[Guest eBPF 和 procfs] --> H2[actrail-sb]
    H2 -->|AF_VSOCK| H3[actrail-vsock-gateway]
    H3 -->|TCP| H4[GatewayIngestRuntime]
    H4 -->|匹配的使用方| H5[Sandbox 插件]
    H4 -->|无匹配使用方| H6[Sandbox Evidence DB]
  end

  subgraph Alerts[外部告警投递]
    A1[规范化告警] --> A2[内置转发插件]
    A2 -->|AF_UNIX| A3[actraild-alert-proxy]
    A3 --> A4[匹配的订阅者]
  end
```

## 脑侧观测

采集与下游解析是否成功相互独立。解析器、导出器或插件的故障被限制在对应的使用方或逻辑流内。

## 手侧观测

摄入后的路由是互斥的。兴趣查询成功且存在匹配插件时，观测会发送到这些插件。查询成功且没有返回匹配项时，观测持久化到 Sandbox Evidence DB。

## 告警投递

告警持久化和外部转发是彼此独立的结果。队列压力或代理断开连接只能丢弃转发副本；不得阻塞或回滚告警生产者。
