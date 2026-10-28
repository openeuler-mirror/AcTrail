状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/semantic_action_runtime/src/llm_pipeline/projection/compact/ # 识别上下文压缩及结果采用，生成压缩动作、证据和采用关系。
├── [x] mod.rs # 声明内部模块并导出观测输入、投影器与输出批次。
├── [x] observation.rs # 接收 provider 解析后的请求历史、上下文关联与响应事实。
│   ├── [x] struct LlmRequestObservation # 携带请求动作 ID、共享历史来源及解析后的上下文。
│   └── [x] struct LlmResponseObservation # 携带响应动作 ID、正文、provider 失败及工具调用事实。
├── [x] projector.rs # 提供请求观测、轨迹事实交付及 trace 清理入口。
│   └── [x] struct ContextCompactProjector # 组合配置、轨迹门、检测器、历史过滤器及作用域状态。
├── [x] projector/ # 协调响应准入、采用确认与增量重试。
│   ├── [x] completion.rs # 保留完成调用、准入成功响应候选并输出显式协议压缩动作。
│   ├── [x] confirmation.rs # 准备采用请求、复用配对证据，确认唯一来源并消费对应候选。
│   └── [x] retry.rs # 新候选及迟到轨迹事实就绪后重试受影响配对，提交确认并清理过期状态。
├── [x] detector.rs # 定义压缩检测器与请求、响应和采用历史的检测上下文。
│   └── [x] trait CompactDetector # 为显式协议和内容流匹配提供统一检测入口。
├── [x] detector/ # 从协议意图、响应工件与采用载体中提取压缩证据。
│   ├── [x] protocol.rs # 识别显式压缩意图及 opaque 工件采用。
│   ├── [x] artifact.rs # 匹配响应工件内容并排除历史正常延续。
│   ├── [x] flow.rs # 遍历载荷快照与响应证据，匹配完整内容或连续片段。
│   ├── [x] json_text.rs # 解码 JSON 文本值并匹配结构化内容。
│   └── [x] rolling.rs # 建立有界滚动指纹索引，查找最长匹配并校验原文。
├── [x] gate.rs # 检查请求时间先后及轨迹 root、fork 边界。
│   ├── [x] struct TrajectorySlot # 通过共享 OnceLock 保存分类器确认的轨迹事实。
│   └── [x] struct TrajectoryBoundaryGate # 读取轨迹事实并判定候选与采用请求的关联资格。
├── [x] history.rs # 在预算内保留历史快照，判断历史延续与压缩来源消费。
│   └── [x] struct HistoryContinuationFilter # 判断历史保留、嵌入延续与压缩来源消费。
├── [x] history/ # 按需物化共享请求历史。
│   └── [x] source.rs # 从 RequestHistoryWitness 的共享快照缓存读取历史内容。
│       └── [x] struct HistoryMaterializer # 根据历史预算及过滤配置构建不可变历史快照。
├── [x] state.rs # 按作用域持有调用与候选状态，维护待定请求及增量重试索引。
│   ├── [x] struct CompactScopeKey # 以 trace、稳定进程身份和 classifier ID 隔离识别状态。
│   ├── [x] struct CompactStateStore # 持有各作用域状态，关联待定请求并收集受影响的采用请求。
│   └── [x] struct CompactScopeState # 保存开放及保留调用、最近完成头、B 候选池和 C 待采用索引。
├── [x] state/ # 管理候选与采用状态的内容、索引和生命周期。
│   ├── [x] entries.rs # 定义请求、最近完成头和压缩候选条目及其适用条件。
│   │   ├── [x] struct PendingCall # 保存请求元数据、共享轨迹槽及可选的 C 待采用状态。
│   │   └── [x] struct CompactionCandidate # B 压缩候选，关联完成响应与来源请求。
│   ├── [x] evidence.rs # 保存采用历史、协议意图、内容流证据及确认结果。
│   │   ├── [x] struct PendingAdoption # C 待采用状态，保存候选配对证据和来源选择。
│   │   └── [x] struct RequestHistoryWitness # 共享请求历史依据及按需构建的不可变快照。
│   ├── [x] pools.rs # 保存 B 候选条目及过期、动作索引，按请求开始时间索引 C。
│   │   ├── [x] struct CompactionCandidatePool # 按容量及 TTL 管理 B 候选和关联索引。
│   │   └── [x] struct PendingAdoptionIndex # 定位 C 待采用请求，其内容保存在 PendingCall.adoption。
│   ├── [x] pending.rs # 迁移开放与保留调用，治理 C 容量和 TTL，退役候选并细化轨迹事实。
│   ├── [x] choice.rs # 按检测器维护配对状态，处理待定、受限证据及歧义并仲裁唯一来源。
│   ├── [x] carrier.rs # 以原始采用载荷为权威来源，按需缓存解码字符串。
│   └── [x] response.rs # 共享不可变响应正文，惰性缓存字符数、JSON 和指纹索引。
├── [x] builder.rs # 构造压缩动作、采用补充、实现关系和输出批次。
│   ├── [x] struct CompactActionBuilder # 构建 agent.context_compaction 动作及显式协议的采用补充。
│   └── [x] struct CompactProjectionBatch # 汇总动作、更新、关系、诊断和已确认采用，交投影协调器消费。
└── [x] diagnostics.rs # 形成预算耗尽、容量回收、候选过期及来源歧义诊断。
    └── [x] struct CompactDiagnostics # 统一压缩识别诊断的上下文和输出。
```
