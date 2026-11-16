# Response Scheduling

Schedule 只控制 lazy protocol frames 的发送节奏，不理解 OpenAI、Anthropic 或 scenario block：

```text
ProtocolFrame iterator
→ ScheduleController
→ timed ProtocolFrame iterator
→ HTTP connection
```

direct JSON response 不拆分，因此不应用 TTFT/TPOT 调度。

## 配置

```text
--ttft-milliseconds
--tpot-milliseconds
```

- TTFT：发送第一个 SSE frame 前的固定延迟；
- TPOT：发送后续每个 SSE frame 前的固定延迟。

Schedule 不读取内容、不估算 token 数，也不区分协议 envelope 和内容 frame。它只做首帧/后续帧两种固定等待，让流式响应具备足够真实的时间形态。

两个时间值都必须有限且非负。默认 TTFT 和 TPOT 都是零，不引入额外 sleep。

调度发生在 scenario runtime 锁外。慢客户端或非零延迟只占用当前 HTTP worker，不阻塞剧本状态管理。
