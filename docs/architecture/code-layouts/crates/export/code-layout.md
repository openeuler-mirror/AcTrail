状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/export/ # 语义观测的订阅、隔离投递与 OTEL 输出适配
├── [x] core/ # 在线发布契约、消费者生命周期与有界投递
│   └── [x] src/ # 导出契约、运行时和队列机制
│       ├── [x] contract/ # 记录视图、输出适配接口和交付结果
│       │   ├── [x] adaptor.rs # 定义语义记录输出能力
│       │   │   └── [x] trait SemanticActionExportAdapter # 将语义记录交给具体输出后端
│       │   ├── [x] record.rs # 提供一次发布的借用记录视图
│       │   │   └── [x] struct SemanticActionExportRecord # 借用 trace 与语义 action 相关内容
│       │   └── [x] error.rs # 表达输出失败和丢弃结果
│       │       ├── [x] struct ExportPublishResult # 汇总一次发布的交付情况
│       │       └── [x] enum ExportDropReason # 区分投递丢弃的原因
│       ├── [x] runtime/ # 按消费者隔离语义发布与终态任务
│       │   ├── [x] publish.rs # 公开发布、动态订阅和关闭入口
│       │   │   ├── [x] struct ExportRuntime # publish_semantic_actions 分发批次；add/remove_observation_consumer 管理订阅
│       │   │   ├── [x] struct SemanticActionExportBatch # 借用 trace、行为、链接和 payload 引用
│       │   │   └── [x] struct ExportPublishReport # 返回丢弃记录与运行失败
│       │   ├── [x] subscription.rs # 维护已加载消费者及各自的运行槽位
│       │   │   └── [x] struct SemanticActionSubscriptionManager # 负责消费者安装、移除与批次分发
│       │   ├── [x] subscription_slot.rs # 封装单个消费者的队列、状态和指标
│       │   │   └── [x] struct ObservationConsumerSlot # 隔离单个消费者的投递与失败
│       │   ├── [x] subscription_worker.rs # 执行排队的观测和 trace 终态任务
│       │   │   ├── [x] enum ObservationWorkItem # 区分观测批次与消费者工作项
│       │   │   └── [x] struct ObservationWorkerControl # 管理消费者工作线程的停止和控制
│       │   ├── [x] subscription_shutdown.rs # 按预算结束消费者并汇总未交付记录
│       │   └── [x] route.rs # 将语义输出适配器接入 best-effort 投递
│       │       ├── [x] trait SemanticActionExportRoute # 定义语义记录的投递路由
│       │       └── [x] struct BestEffortSemanticActionRoute # 组合输出适配器和有界交付
│       └── [x] delivery/ # 可复用的有界后台投递机制
│           └── [x] best_effort.rs # 队列接收、后台写入与有预算关闭
│               ├── [x] trait BestEffortSink # 定义工作线程实际执行的写入能力
│               ├── [x] struct BestEffortDelivery # 持有有界队列并提交投递项
│               └── [x] struct BestEffortShutdown # 协调投递线程关闭
├── [x] factory/ # 根据插件 manifest 创建内置观测消费者
│   └── [x] src/ # 消费者类型选择和配置校验
│       ├── [x] lib.rs # 导出消费者构造和配置校验入口
│       └── [x] builder.rs # build_observation_consumer_from_manifest 创建 OTEL 消费者；validate_observation_consumer_config 校验配置
└── [x] adapters/ # OTEL 传输和编码实现
    ├── [x] otel_codec/ # 将语义记录转换为 OTLP JSON 与 Protobuf
    │   └── [x] src/ # OTLP 数据映射与序列化
    │       ├── [x] service.rs # 构建 OTEL span、属性和行为关联
    │       │   └── [x] struct OtelExportError # 表达导出数据构造失败
    │       ├── [x] serialize.rs # 序列化 OTLP 记录
    │       └── [x] otel_pb.rs # OTLP Protobuf 消息类型
    ├── [x] otel_http/ # 将在线观测批量发送到 OTLP HTTP 端点
    │   └── [x] src/ # HTTP 消费者、发送状态与连接配置
    │       ├── [x] live_http.rs # 组批、压缩、发送和交付结果处理
    │       │   ├── [x] struct OtelHttpObservationConsumer # 接收语义观测并交给 HTTP 投递路由
    │       │   ├── [x] struct HttpBatchSink # 执行实际 HTTP 批次发送
    │       │   └── [x] struct TerminalActionLedger # 跟踪行为终态输出状态
    │       └── [x] config.rs # HTTP 端点、编码、压缩、TLS 和超时设置
    │           ├── [x] struct OtelHttpExporterConfig # 保存 HTTP 导出器配置
    │           ├── [x] enum OtelEncoding # 选择 OTLP 编码
    │           └── [x] struct OtelHttpTlsConfig # 保存 TLS 连接参数
    └── [x] otel_jsonl/ # 将 OTLP JSON 行写入文件或交给 JSON-RPC HTTP 接收端
        └── [x] src/ # JSONL 消费者、输出选择和配置解析
            ├── [x] live_jsonl.rs # 接收语义观测并生成 JSONL 记录
            │   └── [x] struct OtelJsonlObservationConsumer # 将记录交给有界 JSONL 输出路由
            ├── [x] config.rs # 选择输出类型及其专属配置
            │   └── [x] struct OtelJsonlExporterConfig # 保存 JSONL 输出配置
            └── [x] exporter/ # 实际 JSONL 输出后端
                ├── [x] sink.rs # 统一所选后端的写入入口
                │   └── [x] struct ExporterSink # 持有已选择的输出实现
                ├── [x] file.rs # 将 JSONL 记录追加到目标文件
                │   └── [x] struct FileExporterSink # 管理输出文件及写入
                └── [x] json_rpc_http.rs # 通过 JSON-RPC HTTP 交付 JSONL 内容
                    └── [x] struct JsonRpcHttpExporterSink # 管理请求发送和响应处理
```
