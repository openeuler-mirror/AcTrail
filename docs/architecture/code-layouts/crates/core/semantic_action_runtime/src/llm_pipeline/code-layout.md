状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/semantic_action_runtime/src/llm_pipeline/ # 从明确来源的明文段生成 LLM action、内容写入和请求 lineage。
├── [x] mod.rs # 公开 facade 与 codec 接口，隔离传输、provider 和投影实现。
├── [x] facade/ # 统一载荷、事件、缺口和 trace 收尾入口。
│   ├── [x] pipeline.rs # 持有协议状态、codec 注册表及投影协调器。
│   │   └── [x] struct LiveLlmProjector # 以 LlmActionPipeline 名称导出，拥有 HTTP/1、HTTP/2、WebSocket 状态。
│   ├── [x] router.rs # 选择输入流对应的协议并维护协议路由身份。
│   ├── [x] payload.rs # 执行载荷分派和本次协议输出汇总。
│   └── [x] output.rs # 组成语义批次，交 live runtime 与 recording 使用。
├── [x] transport/ # 保留消息与帧边界，不承担 provider 语义分类。
│   ├── [x] http1/ # 增量恢复 HTTP/1 消息边界、transfer framing 和内容编码。
│   ├── [x] http2/ # 按帧头、连接状态与 stream ID 解码 HTTP/2。
│   │   └── [x] decoder.rs # 驱动帧解析并输出 typed HTTP/2 事件。
│   │       └── [x] struct Http2Decoder # 保存增量解码状态，管理帧与头部恢复。
│   └── [x] websocket/ # 处理 WebSocket 帧、消息重组及连接中止。
├── [x] assembly/ # 按流身份组织分片、方向与协议输入。
│   ├── [x] router/ # 流关联键、方向与 assembly 容量约束。
│   │   ├── [x] identity.rs # 建立段来源到流组和方向的关联值。
│   │   │   └── [x] struct PayloadStreamGroupKey # 以进程、来源与流坐标区分 assembly 状态。
│   │   └── [x] limits.rs # 将配置转换为 assembly 状态预算和重置原因。
│   ├── [x] http2/ # 将 HTTP/2 多 stream 正文交给相应语义消费者。
│   └── [x] plain/ # 保存普通明文输入及有界正文缓冲。
├── [x] http1/ # 拥有 HTTP/1 协议生命周期并协调请求响应与正文消费。
│   └── [x] protocol.rs # 将 HTTP/1 decoder 输出路由到协议观测及内容投影。
├── [x] http2/ # 管理 HTTP/2 连接和 stream 的内容生命周期与诊断。
│   ├── [x] protocol.rs # 承接明文输入、流关闭及 trace 清理。
│   └── [x] state.rs # 保存活跃 stream 的语义解析状态。
├── [x] http/ # 定义供 provider 解析的共享 HTTP 请求响应值。
├── [x] websocket/ # 将 WebSocket 消息与 provider 会话关联并生成语义输出。
├── [x] websocket_source/ # 验证和分配 WebSocket 消息的来源关联。
├── [x] stream/ # 处理 SSE 增量边界、响应累积、分类和结束原因。
│   ├── [x] sse_framer/ # 在任意输入分片之间保持 SSE 行与事件边界。
│   ├── [x] classifier/ # 判断流式内容归属及相应解析策略。
│   ├── [x] response/ # 累积响应语义，跟踪内容和 usage 的完成状态。
│   └── [x] finalizer/ # 将正常结束、缺口和预算耗尽转换为局部结束结果。
├── [x] provider/ # 内置 provider 解码和动态 LLM codec 扩展。
│   ├── [x] registry/ # 统一请求解析协议、识别事实及通用 JSON 请求处理。
│   │   └── [x] request.rs # 定义 provider 请求输入和解析器边界。
│   │       └── [x] trait LlmRequestParser # 将已恢复的请求解析成 provider 语义结果。
│   ├── [x] codec/ # 动态插件的注册、调用及解析失败诊断。
│   │   └── [x] adapter.rs # 对外提供 codec 注册表并适配请求和 SSE 调用。
│   │       └── [x] struct LlmCodecRegistry # 持有已加载 codec 和修订号，支持 register/unregister。
│   ├── [x] anthropic/ # 解析 Anthropic 请求与流式响应形态。
│   ├── [x] openai_chat/ # 解析 OpenAI Chat 风格消息、工具与 usage。
│   ├── [x] openai_responses/ # 解析 Responses API 输入、输出及响应关联。
│   ├── [x] driver/ # 驱动 provider 解析并组合协议层已确认事实。
│   └── [x] structured_json/ # 增量处理 provider 内容中的结构化 JSON。
└── [x] projection/ # 将解析事实转换为动作、内容、关系和轨迹记录。
    ├── [x] orchestrator/ # 协调动作变更、内容写入和延迟轨迹分配。
    │   └── [x] orchestrator.rs # 组合 projector 并统一输出批次。
    │       └── [x] struct ProjectionCoordinator # 拥有动作投影状态，协调请求响应和轨迹生命周期。
    ├── [x] projector/ # 请求、响应及动作版本的语义投影。
    │   ├── [x] request.rs # 从 provider 请求生成动作属性、内容与历史输入。
    │   │   ├── [x] struct ProjectedLlmRequestHistory # 携带 action 身份、classifier 和规范化历史。
    │   │   └── [x] enum ProviderContextReference # 区分无 provider 关联、根上下文及 previous_response_id。
    │   └── [x] projector.rs # 管理动作记录及待解析的 trajectory 归属。
    │       └── [x] struct ActionProjector # 持有动作状态、版本和有限待分配记录。
    ├── [x] trajectory/ # 在 trace、稳定进程身份和 classifier 作用域内归类请求历史。
    │   └── [x] classifier/ # 历史前缀和 provider response ID 关联算法。
    │       ├── [x] implementation.rs # 拥有各作用域的轨迹、候选及容量清理状态。
    │       │   └── [x] struct TrajectoryClassifier # 输出 root、append、fork 或 duplicate 分配及 lineage。
    │       └── [x] implementation/ # provider ID 关联与延迟请求释放。
    │           └── [x] provider.rs # 用已确认 response ID 绑定解析延续，限制歧义与过期状态。
    ├── [x] retention/ # 根据语义保留配置形成正文、shape、工具结果和摘要。
    │   └── [x] evidence.rs # 将输入 payload 片段及其范围转换为紧凑动作证据。
    └── [x] links/ # 建立 LLM 与 HTTP 请求响应之间的明确来源关系。
```
