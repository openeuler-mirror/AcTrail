状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/recording/ # 将观测批次写入主存储，并分别报告持久化与在线发布结果
└── [x] runtime/ # 记录事务、语义批次与导出协调
    └── [x] src/ # 记录运行时实现与公开写入入口
        ├── [x] lib.rs # 导出写入器、批次、写入会话和交付报告
        ├── [x] writer.rs # 聚合 trace、membership、诊断与语义记录的写入操作
        │   └── [x] struct RecordingWriter # 借用 StorageBackend；persist_live_events_then_export、write_session_then_export 协调写入与发布
        ├── [x] commit.rs # 组织一次观测批次的事务提交与独立导出
        │   └── [x] struct ObservedRecordCommitCoordinator # 借用主存储与 ExportRuntime；记录存储与语义交付结果后继续处理可发布批次
        ├── [x] observed.rs # 聚合进程、trace、membership、payload、事件及语义写入
        │   ├── [x] struct ObservedRecordBatch # 保存一次提交中的观测记录与语义批次
        │   ├── [x] struct ObservedRecordWriteSession # persist_event、persist_payload_segment、persist_semantic_actions 在当前事务内收集写入
        │   └── [x] struct TraceStateRecord # 携带待保存的 trace 状态
        ├── [x] transaction.rs # 将记录事务的提交和失败回滚交给存储事务接口
        │   └── [x] struct RecordingTransaction # begin 创建事务；commit_or_rollback 根据写入结果完成事务
        ├── [x] delivery.rs # 汇总每个交付阶段的失败与存储结果
        │   ├── [x] struct DeliveryReport # storage_succeeded 查询持久化结果，into_failures 取出失败列表
        │   ├── [x] struct DeliveryFailure # 保存失败阶段和错误信息
        │   └── [x] enum DeliveryFailureKind # 区分存储、可选属性补充拒绝和导出等交付阶段
        ├── [x] diagnostics.rs # 将交付错误与丢弃信息写入诊断记录
        └── [x] semantic/ # 语义 action、link、正文与 lineage 的批量持久化和发布
            ├── [x] batch.rs # 拥有或借用语义批次及其附属内容
            │   ├── [x] struct SemanticActionBatch # 收集 action、更新、link、正文和 lineage；按存储与导出需要提供视图
            │   └── [x] struct SemanticActionRecordBatch # 借用批次记录并校验 trace 归属
            ├── [x] recorder.rs # 将批次中的语义记录写入主存储并报告属性补充拒绝
            │   └── [x] struct SemanticActionRecorder # 借用存储后端并执行语义记录保存
            ├── [x] persistence.rs # 汇集语义写入并统一提交
            │   └── [x] struct SemanticActionPersistenceAccumulator # 保存待刷新的语义记录
            ├── [x] publication.rs # 组织一次语义发布需要的数据
            │   └── [x] struct SemanticActionPublication # 持有用于在线发布的语义批次
            ├── [x] export.rs # 查询 trace 元信息并构造在线导出记录
            │   ├── [x] trait TraceRecordLookup # 为导出提供 trace 记录查询
            │   └── [x] struct SemanticActionExportRecorder # 将语义批次交给导出运行时并保存失败诊断
            └── [x] error.rs # 记录阶段的错误模型
                └── [x] struct RecordingError # 保存阶段名称和错误信息
```
