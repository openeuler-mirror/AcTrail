# Protocol Adapter

Protocol adapter 是 scenario 与 wire protocol 之间的唯一转换边界：

```text
request JSON → ScenarioRequest
ScenarioEmission → direct body 或 lazy SSE frames
request error → 对应协议的 error envelope
```

HTTP connection 和 scenario runtime 不解析协议字段。

## 接口

`protocol/interface.py` 定义：

```python
class ProtocolAdapter(ABC):
    def decode_request(self, document: dict) -> ScenarioRequest: ...
    def encode_response(
        self,
        request: ScenarioRequest,
        emission: ScenarioEmission,
        default_model: str,
    ) -> ProtocolResponse: ...
    def encode_error(
        self,
        status: int,
        code: str,
        message: str,
    ) -> ProtocolResponse: ...
```

`ProtocolResponse` 只能包含 direct body 或 lazy frame iterator 之一。`ProtocolFrame` 只保存实际 wire payload。

## 内置协议

| Adapter | 路径 | direct | SSE |
| --- | --- | --- | --- |
| OpenAI Chat Completions | `/chat/completions`、`/v1/chat/completions` | JSON completion | `data:` events，以 `[DONE]` 结束 |
| Anthropic Messages | `/messages`、`/v1/messages` | JSON message | `message_start` 到 `message_stop` |

OpenAI adapter 负责 reasoning_content、tool_calls、finish_reason 和 usage-only chunk。Anthropic adapter 负责 thinking、signature、tool_use、stop_reason 和 usage events。

Wire response id、tool-call id 和 Anthropic signature 都由 adapter 根据 emission index 生成，scenario 模板不携带协议身份。

## Registry

`ProtocolRegistry` 在启动时构造并冻结：

- protocol name 必须唯一；
- 不使用 import-time 注册或目录扫描。

`server_core/api_endpoints.py` 从 registry 构造 endpoint 路由，启动时拒绝重复 path，请求热路径通过字典 O(1) 查找 adapter。

增加协议时必须提供真实 request/response contract、adapter 和真实客户端 E2E。没有明确 wire contract 的 ChatGPT adapter 不创建占位实现。
