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
    def input_tokens(self, document: dict) -> int: ...
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

adapter 从外部请求构造 `ScenarioRequest`，并以规范化 JSON 的字符数作为本地测试用的 input-token 近似值。该值描述当前请求，不由 scenario 配置，也不在 runtime 中跨请求累计。

## 内置协议

| Adapter | 路径 | direct | SSE |
| --- | --- | --- | --- |
| OpenAI Chat Completions | `/chat/completions`、`/v1/chat/completions` | JSON completion | `data:` events，以 `[DONE]` 结束 |
| Anthropic Messages | `/messages`、`/v1/messages` | JSON message | `message_start` 到 `message_stop` |

OpenAI adapter 负责 reasoning_content、tool_calls、finish_reason、usage-only chunk，以及 OpenAI function tools 的名称和 parameters 提取。Anthropic adapter 负责 thinking、signature、tool_use、stop_reason、usage events，以及 Anthropic tools 的名称和 input_schema 提取。

请求工具定义进入协议无关的 `ToolDefinition`。scenario 仍产生普通的规范工具调用；位于 scenario 与协议编码之间的抽象 Alias Converter 将其转换为请求允许的实际名称和参数。Wire response id、tool-call id 和 Anthropic signature 都由 adapter 根据 emission index 生成，scenario 模板不携带协议身份。

## Registry

`ProtocolRegistry` 在启动时构造并冻结：

- protocol name 必须唯一；
- 不使用 import-time 注册或目录扫描。

`server_core/api_endpoints.py` 从 registry 构造 endpoint 路由，启动时拒绝重复 path，请求热路径通过字典 O(1) 查找 adapter。

增加协议时必须提供真实 request/response contract、adapter 和真实客户端 E2E。没有明确 wire contract 的 ChatGPT adapter 不创建占位实现。
