# 关键数据流

AcTrail 的观测、治理、资源采样、隔离观测和告警投递各自具有明确入口与故障边界。图中箭头表示数据或决策传递；交互顺序见[运行时序](../sequence-graphs.md)。

```mermaid
flowchart LR
  subgraph Brain[Brain-side 观测]
    B1[内核事件或明文] --> B2[摄入与身份关联]
    B2 --> B3[协议和语义投影]
    B3 --> B4[Recording]
    B4 --> B5[主存储]
    B4 --> B6[导出使用方]
  end
  subgraph Control[操作治理]
    C1[用户态控制通知] --> C2[身份与归属]
    C2 --> C3[规则与 control-decider]
    C3 --> C4[操作响应与审计]
    K1[已编译 LSM 规则] --> K2[内核操作裁决]
    K2 --> K3[异步审计]
  end
  subgraph Resource[资源观测]
    R1[Host / Workload 采样] --> R2[资源样本与告警评估]
    R2 --> R3[资源存储后端]
    R4[cgroup 生命周期] --> R5[主存储控制记录]
  end
  subgraph Hand[Hand-side 观测]
    H1[Guest eBPF 和 procfs] --> H2[actrail-sb]
    H2 -->|AF_VSOCK| H3[actrail-vsock-gateway]
    H3 -->|TCP| H4[GatewayIngestRuntime]
    H4 -->|逐条匹配| H5[Sandbox 插件]
    H4 -->|逐条无兴趣| H6[Sandbox Evidence DB]
    H5 -->|提交告警| H7[Sandbox Alert DB]
  end
  subgraph Alerts[外部告警投递]
    A1[规范化告警] --> A2[内置转发插件]
    A2 -->|AF_UNIX| A3[actraild-alert-proxy]
    A3 --> A4[匹配的订阅者]
  end
```

## 观测与治理

采集和下游解析各自处理失败。解析器、导出器或插件的故障限制在对应使用方或逻辑流。主存储与导出分别报告交付结果，存储失败后仍可尝试发布。

用户态通知治理在待裁决操作边界取得上下文并返回决策。LSM 使用预先发布到内核的规则完成操作裁决，用户态异步读取审计。审计进入对应观测路径，控制响应按所选 backend 和策略执行。

## 资源观测

资源样本使用组件拥有的 database、remote 或 hybrid 后端。cgroup 绑定及控制完成记录通过主存储持久化，样本投递为 best-effort。指标语义与生命周期见[资源设计](../components/resource-profiling/design.md)。

## 手侧观测

路由互斥粒度是单条 observation。同一批中，兴趣查询成功且有匹配插件的项投递插件，无匹配使用方的项写入 Sandbox Evidence DB。兴趣查询失败作为局部错误处理，不等同于无兴趣。

## 告警投递

告警持久化和外部转发分别报告结果。队列压力或代理断开可以丢弃转发副本；故障限制在转发使用方内。
