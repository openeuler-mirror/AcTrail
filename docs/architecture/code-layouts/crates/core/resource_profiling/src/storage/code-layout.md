状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/resource_profiling/src/storage/ # 资源存储子模块；拥有写入 trait、Database 扩展 trait、Factory、远端、本地和 Hybrid 实现。
├── [x] config/ # 选择资源存储后端，并委托对应模块解析配置。
│   ├── [x] mod.rs # 声明并导出根后端配置。
│   └── [x] resource_storage_config.rs # 解析后端种类，将所选子表交给所属模块解析。
│       ├── [x] enum ResourceStorageBackendKind # remote、database、hybrid。
│       ├── [x] struct RawConfig # 接收后端种类与原始子表。
│       └── [x] struct ResourceStorageConfig # 组合 backend 与 remote/database/hybrid 专属配置。
├── [x] contracts/ # 定义资源样本批量写入接口与局部错误。
│   ├── [x] mod.rs # 声明并导出资源写入契约。
│   ├── [x] resource_storage_backend.rs # 定义统一的资源样本批量写入能力。
│   │   └── [x] trait ResourceStorageBackend # 接收 Host/Workload 样本批次，返回本次写入结果；调用方经此接口使用任意后端。
│   └── [x] resource_storage_error.rs # 表示单次资源写入与配置构造错误。
│       └── [x] struct ResourceStorageError # 携带资源写入或配置错误的说明，供调用方局部记录。
├── [x] database/ # 扩展资源写入接口，提供本地查询、删除与保留策略。
│   ├── [x] config/ # 选择数据库种类并解析专属配置。
│   │   ├── [x] database_storage_config.rs # 解析数据库种类，并委托所选数据库解析专属配置。
│   │   │   ├── [x] enum DatabaseStorageBackendKind # sqlite。
│   │   │   ├── [x] struct DatabaseStorageConfig # 组合数据库 backend 与对应子配置。
│   │   │   └── [x] struct RawConfig # 接收数据库种类与原始子表。
│   │   └── [x] mod.rs # 声明并导出数据库后端选择配置。
│   ├── [x] database_storage_backend.rs # 扩展资源写入契约，定义本地数据库操作。
│   │   └── [x] trait DatabaseStorageBackend: ResourceStorageBackend # 提供资源查询与按时间范围删除能力；参数使用资源模型和查询条件。
│   ├── [x] database_storage_factory.rs # 根据数据库 enum 构造具体数据库后端。
│   │   └── [x] struct DatabaseStorageFactory # 委托选定数据库配置解析与实例构造，返回 DatabaseStorageBackend 实例。
│   ├── [x] mod.rs # 声明并导出数据库扩展接口、配置与 Factory。
│   ├── [x] resource_query.rs # 定义资源类别、身份、时间范围及查询数量限制。
│   │   ├── [x] enum ResourceKind # 定义资源类别、身份、时间范围及查询数量限制。
│   │   └── [x] struct ResourceQuery # 资源身份、Host/Workload 类别、时间范围与结果数量限制。
│   └── [x] sqlite/ # 在独立资源数据库中保存、查询和清理样本。
│       ├── [x] config/ # 配置资源 SQLite 路径、保留期限和清理预算。
│       │   ├── [x] mod.rs # 声明并导出 SQLite 配置与保留策略。
│       │   ├── [x] resource_retention_config.rs # 解析资源样本保留期限、清理周期与单轮预算。
│       │   │   └── [x] struct ResourceRetentionConfig # 拥有本地资源数据库的保留策略、默认值与校验。
│       │   └── [x] resource_storage_sqlite_config.rs # 解析资源数据库文件路径及保留策略，提供数据库及 WAL/SHM 文件清单。
│       │       └── [x] struct ResourceStorageSqliteConfig # 拥有 SQLite 路径和 retention 子配置。
│       ├── [x] mod.rs # 声明并导出 SQLite 资源存储实现。
│       ├── [x] resource_storage_sqlite_backend.rs # 管理独立资源数据库的连接、写入、查询与删除。
│       │   └── [x] struct ResourceStorageSqliteBackend # 实现 ResourceStorageBackend 和 DatabaseStorageBackend；使用专属资源表保存 Host/Workload 样本。
│       ├── [x] retention.rs # 执行有界的过期样本清理。
│       │   └── [x] struct ResourceRetention # 依据 SQLite 自身 retention 配置调度删除。
│       └── [x] schema.sql # 定义资源样本表、时间与资源身份索引。
├── [x] hybrid/ # 将同一批资源样本投递到独立排队的多个子后端。
│   ├── [x] child_dispatcher.rs # 为每个子后端管理独立的有界队列和写入任务。
│   │   └── [x] struct ChildDispatcher # 共享只读样本批次；队列满时丢弃该目标本批请求，写入失败或超时局部记录，继续处理后续批次。
│   ├── [x] config/ # 配置子后端列表、独立队列容量及写入超时。
│   │   ├── [x] hybrid_child_config.rs # 识别子后端种类并委托所属模块解析子配置。
│   │   │   ├── [x] enum HybridChildBackendKind # remote、database。
│   │   │   ├── [x] struct HybridChildConfig # 子后端名称、种类及 remote/database 专属配置。
│   │   │   └── [x] struct RawConfig # 接收子后端标识、种类与原始子表。
│   │   ├── [x] hybrid_resource_storage_config.rs # 解析子后端列表、每个目标的队列容量与单批写入超时。
│   │   │   ├── [x] struct HybridResourceStorageConfig # 拥有 backends 列表与 best-effort 分发参数。
│   │   │   └── [x] struct RawConfig # 解析子后端列表、每个目标的队列容量与单批写入超时。
│   │   └── [x] mod.rs # 声明并导出组合配置与子后端描述。
│   ├── [x] hybrid_resource_storage_backend.rs # 将同一批样本 best-effort 分发给多个子后端。
│   │   └── [x] struct HybridResourceStorageBackend # 实现 ResourceStorageBackend；持有一组子后端接口，独立调度各目标写入并记录局部失败。
│   ├── [x] hybrid_resource_storage_factory.rs # 解析子后端描述并调用 remote/database Factory。
│   │   └── [x] struct HybridResourceStorageFactory # 依赖 RemoteResourceStorageFactory 与 DatabaseStorageFactory，构造子后端后统一注入 HybridResourceStorageBackend。
│   └── [x] mod.rs # 声明并导出组合后端、配置与 Factory。
├── [x] mod.rs # 声明并导出存储契约、根配置、Factory 和 remote/database/hybrid 子模块。
├── [x] queued_resource_storage_backend.rs # 为单目标后端提供有界后台写入；队列满时丢弃本批，错误局部记录。
│   └── [x] struct QueuedResourceStorageBackend # 为单目标后端提供有界后台写入；队列满时丢弃本批，错误局部记录。
├── [x] remote/ # 将资源样本编码后通过有超时的 HTTP 请求外发。
│   ├── [x] config/ # 选择外发协议并配置接收地址、请求头和超时。
│   │   ├── [x] mod.rs # 声明并导出远端配置。
│   │   └── [x] remote_resource_storage_config.rs # 解析外发协议、接收地址、请求头与请求超时。
│   │       ├── [x] enum RemoteResourceProtocol # prometheus_remote_write、influxdb_line_protocol。
│   │       ├── [x] struct RawConfig # 解析外发协议、接收地址、请求头与请求超时。
│   │       └── [x] struct RemoteResourceStorageConfig # 拥有远端连接和外发参数的默认值与校验。
│   ├── [x] mod.rs # 声明并导出远端后端、配置与 Factory。
│   ├── [x] protocol/ # 映射资源身份与指标，编码 InfluxDB 和 Prometheus 请求。
│   │   ├── [x] influxdb_line_protocol.rs # 将样本转换为 InfluxDB line protocol 请求。
│   │   │   └── [x] struct InfluxdbLineProtocolClient # 管理 measurement、tag、field、时间戳映射与 HTTP 外发。
│   │   ├── [x] metric_batch.rs # 将 Host/Workload 样本映射为带来源与身份标签的指标点。
│   │   │   ├── [x] struct MetricBatch # 将 Host/Workload 样本映射为带来源与身份标签的指标点。
│   │   │   └── [x] struct MetricPoint # 将 Host/Workload 样本映射为带来源与身份标签的指标点。
│   │   ├── [x] mod.rs # 声明远端协议编码与请求实现。
│   │   └── [x] prometheus_remote_write.rs # 将样本转换为 Prometheus Remote Write 请求。
│   │       ├── [x] struct Label # 将样本转换为 Prometheus Remote Write 请求。
│   │       ├── [x] struct PrometheusRemoteWriteClient # 管理指标名称、单位、标签映射与 Remote Write 外发。
│   │       ├── [x] struct Sample # 将样本转换为 Prometheus Remote Write 请求。
│   │       ├── [x] struct TimeSeries # 将样本转换为 Prometheus Remote Write 请求。
│   │       └── [x] struct WriteRequest # 将样本转换为 Prometheus Remote Write 请求。
│   ├── [x] remote_resource_storage_backend.rs # 向第三方资源接收端外发样本。
│   │   └── [x] struct RemoteResourceStorageBackend # 实现 ResourceStorageBackend；持有自身协议客户端，执行有超时的批量外发。
│   └── [x] remote_resource_storage_factory.rs # 按远端配置创建协议客户端与外发后端。
│       └── [x] struct RemoteResourceStorageFactory # 消费 RemoteResourceStorageConfig，构造 RemoteResourceStorageBackend。
└── [x] resource_storage_factory.rs # 按根配置调用对应后端 Factory。
    └── [x] struct ResourceStorageFactory # 装配 ResourceStorageBackend；具体协议、数据库与子后端列表由所属 Factory 处理。
```
