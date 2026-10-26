状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/config/src/daemon/agent/ # HTTP、WebSocket 和语义内容保留配置。
├── [x] http1.rs # 定义 HTTP/1 消息解析的缓冲和证据范围上限。
│   └── [x] struct Http1ProtocolConfig # 保存消息缓冲容量和证据范围数量，并提供默认值。
├── [x] websocket.rs # 定义 WebSocket 传输保留策略及模型会话容量。
│   ├── [x] struct L2WebSocketRetention # 配置连接、帧、消息、解码内容和证据范围的上限。
│   └── [x] struct LlmWebSocketConfig # 限制每个连接的会话通道和待处理请求数量。
└── [x] retention.rs # 根据各语义层开关和内容归属策略决定实际保留内容。
    └── [x] impl SemanticRetentionConfig # 查询模型调用、SSE、HTTP、HTTP/2 和传输载荷的保留行为。
        ├── [x] llm_trajectory_enabled # 判断模型调用层和标准请求块是否支持轨迹记录。
        ├── [x] sse_event_content_for_llm_response # 根据模型响应消费情况选择 SSE 事件内容。
        ├── [x] http_body_content_for_http_message # 根据模型层消费和内容归属策略选择 HTTP 正文。
        └── [x] retain_transport_payload_body # 根据高层消费情况决定是否保留传输载荷正文。
```
