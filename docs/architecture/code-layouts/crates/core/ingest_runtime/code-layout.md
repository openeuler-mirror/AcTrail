状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/ingest_runtime/ # 将 collector 事件转为带身份、策略及 provider 分类的领域事件。
└── [x] src/ # 单事件摄入步骤；应用提供精确归属及记录 ID。
    ├── [x] lib.rs # 组合规范化、观测策略与 provider 分类，返回事件和诊断。
    │   ├── [x] struct IngestPipeline # 持有策略评估器并借用 provider classifier，组织一次处理。
    │   │   └── [x] process # 缺少匹配时仅产身份诊断；策略允许后生成领域事件及可选标签事件。
    │   ├── [x] struct IngestMatch # 由调用方确认的 trace、进程身份和可选父身份。
    │   └── [x] struct IngestOutcome # 一次摄入生成的事件与诊断集合，交调用方继续投影和记录。
    ├── [x] match_event.rs # 根据稳定身份解析事件对应的 membership。
    ├── [x] normalize.rs # 将 raw collector 内容与已匹配身份组装为 DomainEvent。
    ├── [x] policy_gate.rs # 对观测内容应用记录策略，形成允许、丢弃等结果。
    ├── [x] classify.rs # 从已规范化事件提取 provider 证据并生成标签事件。
    └── [x] diagnostics.rs # 为身份匹配失败及观测策略决定生成诊断记录。
```
