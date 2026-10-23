状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/cluster/ # actrailcluster；本地 trace 打包上报与中心接收索引的应用组合
├── [x] Cargo.toml # 配置、存储、JSON graph 导出与 SQLite 索引依赖
└── [x] src/ # bundle 编解码、HTTP 上传接收和周期上报
    ├── [x] bin/ # 集群命令可执行入口
    │   └── [x] actrailcluster.rs # 将参数交给集群应用并返回执行结果
    ├── [x] lib.rs # 分派 pack/upload/serve/import/list；装配本地只读存储与中心索引
    │   ├── [x] struct Cli # 保存 operator 配置路径与子命令
    │   ├── [x] enum Command # 区分单次打包、单次或循环上传、中心监听及离线导入查询
    │   ├── [x] struct BundleDocument # 拥有 manifest、JSON graph 和可选 SQLite snapshot 字节
    │   ├── [x] struct BundleManifest # 记录 bundle 格式、生成时间、节点身份、trace 信息和计数
    │   ├── [x] struct BundleTrace # 同时保存外部 trace_uid 与本地 local_trace_id 及生命周期摘要
    │   ├── [x] struct BundleCluster # 保存集群和节点的标识、名称与地址
    │   ├── [x] struct BundleCounts # 汇总事件、动作、关联、payload 与诊断数量
    │   ├── [x] struct ReportState # 独立 SQLite 连接持有各 trace 的上传状态、指纹和失败信息
    │   │   ├── [x] open # 准备上报状态表并读取已有上传记录
    │   │   └── [x] mark_uploaded # 在上传成功后保存 bundle 指纹和服务端回复
    │   ├── [x] struct UploadSummary # 返回上传成功、未改变跳过及失败数量
    │   └── [x] struct HttpRequest # 接收中心请求的方法、路径、请求头和正文
    └── [x] upload_loop.rs # 按配置周期重载并执行 upload_once，处理信号和实例互斥
        └── [x] struct UploadLoopLock # 持有循环上报的进程锁，释放时结束该实例的独占权
```
