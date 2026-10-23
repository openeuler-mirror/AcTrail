状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/semantic_action_runtime/ # 将低层事件和明文协议恢复为动作、关系、内容与 lineage，输出交由 recording 处理。
└── [x] src/ # 实时动作投影、共享 HTTP 观测和 LLM 协议流水线。
    ├── [x] lib.rs # 导出 LiveSemanticActionRuntime、HTTP 观测值和 LLM codec 扩展入口。
    ├── [x] http_observation.rs # 在协议解析与 daemon 应用协议服务之间传递消息边界。
    │   ├── [x] enum Http1MessageEvent # 表达消息开始、正文、结束和中止，保留来源身份。
    │   └── [x] struct Http2FrameObservation # 携带可供独立 HTTP/2 观测消费的帧事实。
    ├── [x] live/ # 按领域拥有实时 projector 状态，汇总当前批次的语义输出。
    │   ├── [x] runtime.rs # 组合 agent、command、file、LLM、MCP、tool 和 link projector。
    │   │   └── [x] struct LiveSemanticActionRuntime # 拥有领域 projector，统一处理事件和载荷生命周期。
    │   │       ├── [x] observe_event # 将 DomainEvent 分派到相关 projector 并返回语义批次。
    │   │       └── [x] finish_file_io_batch # 结束本批文件 IO 汇总并生成动作关联。
    │   ├── [x] runtime/ # 共享输出结构及载荷消费门面。
    │   │   ├── [x] output.rs # 聚合动作、关系、内容、lineage 和诊断的交付值。
    │   │   └── [x] payload.rs # 将载荷段、缺口和收尾通知交给协议及 MCP 消费者。
    │   ├── [x] agent.rs # 从进程与输入事实识别 Agent 会话和任务活动。
    │   │   └── [x] struct AgentProjector # 保存待匹配 exec 意图、用户输入和 Agent 投影状态。
    │   ├── [x] command.rs # 从命令执行与进程退出形成 command 动作。
    │   │   └── [x] struct CommandProjector # 关联命令生命周期及 MCP 调用证据。
    │   ├── [x] file/ # 文件访问、目录枚举、IO 摘要及路径集合投影。
    │   │   ├── [x] projection.rs # 导出文件投影入口并组织各类事件投影实现。
    │   │   ├── [x] projection/ # 按访问、枚举、bulk read 和 IO 类别实现投影。
    │   │   │   └── [x] access.rs # 持有文件投影配置，分派文件事件并汇总本批 IO。
    │   │   │       └── [x] struct FileAccessProjector # 将文件事件合并为访问与 IO 动作，按配置保留事实。
    │   │   └── [x] shared/ # 共享 FD 身份、事件字段和路径集合处理。
    │   ├── [x] http_exchange.rs # 在同一协议流中维护请求与响应的唯一配对。
    │   │   └── [x] struct HttpExchangeTracker # 拥有待配对请求、响应及流隔离状态。
    │   ├── [x] mcp/ # 从 stdio 帧恢复 JSON-RPC 请求、响应及 MCP 内容。
    │   │   └── [x] projector.rs # 组合 MCP framing、会话和内容投影。
    │   │       └── [x] struct LiveMcpProjector # 拥有 MCP 会话与输入缓冲，输出动作和局部诊断。
    │   ├── [x] tool/ # 关联 LLM 声明的工具调用、返回结果和 Agent invocation。
    │   │   ├── [x] projector.rs # 工具投影入口，消费 LLM 动作及请求 lineage。
    │   │   │   └── [x] struct ToolInteractionProjector # 拥有关联器和有界工具状态，生成工具动作及关系。
    │   │   └── [x] internal/ # 将声明、关联、状态和动作输出集中到各自职责。
    │   │       ├── [x] declaration/ # 从模型响应中提取已声明的 tool call。
    │   │       ├── [x] correlation/ # 匹配工具结果及 Agent 调用证据。
    │   │       ├── [x] state/ # 保存待关联工具记录及容量受限的索引。
    │   │       └── [x] emission/ # 从确认事实生成动作、link 和诊断。
    │   └── [x] links/ # 按动作类别维护已确认的父子与协议关联。
    │       └── [x] projector.rs # 组合 agent、command、HTTP、LLM 与 SSE 关系投影。
    │           └── [x] struct ActionLinkProjector # 依据动作身份生成关联，收尾时释放 trace 局部状态。
    └── [x] llm_pipeline/ # 恢复传输协议、解析 provider 内容并生成 LLM 语义与轨迹。
```
