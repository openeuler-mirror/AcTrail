# LLM Responses Request Projector 设计文档

本目录收录 AcTrail 对多种 LLM request JSON 形态进行识别、无损保留、统一投影并交付 Web 展示的设计规范。目录名中的 `response` 指 Responses family 协议 namespace；本文讨论的 `response.create` 是客户端发出的 request envelope，不是模型返回的 `llm.response`。

## 阅读顺序

1. [LLM Request Protocol Projector 规范](llm-request-protocol-projector.md)
   - 定义原始协议、统一 request item 模型、projector contract、选择结果和失败语义。
   - 明确 Chat Completions、Responses 和 Codex Responses Lite 的适配边界。
   - 规定 canonical raw content 与 normalized projection 的双轨职责。

2. [当前实现路径地图](current-implementation-paths.zh.md)
   - 标识当前 WebSocket adapter、request registry、canonical block retention、Web API 和前端解析路径。
   - 解释当前 registry 只返回 classifier/model，而协议结构解析泄漏到前端的问题。

3. [目标文件路径](targeted-file-paths.zh.md)
   - 定义重构后的 contract、registry、dialect projector、共享 item decoder、存储/API 和前端路径。
   - 明确路径 namespace、依赖方向和迁移完成条件。

## 文档职责

| 文档 | 性质 | 更新时机 |
| --- | --- | --- |
| `llm-request-protocol-projector.md` | 规范性目标设计 | projector contract、统一模型、选择或失败语义变化时 |
| `current-implementation-paths.zh.md` | 当前代码路径快照 | 实现迁移、API 或前端消费路径变化时 |
| `targeted-file-paths.zh.md` | 目标源码路径与 namespace 规范 | 目标模块边界、contract 分层或 dialect tree 变化时 |
| `README.zh.md` | 目录索引 | 新增、删除或重命名本目录文档时 |

当前实现不能仅因为被记录在路径地图中就获得规范地位；发生冲突时，应先依据目标规范判断是实现需要迁移，还是规范需要显式修订。
