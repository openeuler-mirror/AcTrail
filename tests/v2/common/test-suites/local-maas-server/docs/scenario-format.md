# Scenario Generator 格式

每个 JSON 文件本身就是一个 generator 对象。文件相对路径是 scenario id；没有额外的 `version`、`name` 或顶层 `generator` 包装。

根 generator 必须包含英文 `description`，说明该 scenario 的返回行为和用途。它只属于 scenario，不会传入下层 generator：

```json
{
  "description": "Returns two assistant messages in order and then exhausts.",
  "type": "response",
  "response": {
    "blocks": [{"type": "message", "text": "Done"}]
  }
}
```

服务在启动时读取并完整校验模板，但不展开 sequential、loop 或 random。一次服务启动创建一个 lazy iterator，收到 MaaS 请求时取得下一份 response。

播放采用 at-most-once 语义：请求匹配 expectation 并取得 response 后立即推进 iterator；后续协议编码、SSE 调度或网络发送失败不会回滚。单个服务进程只供一个串行 Agent 或测试使用，并行 case 分别启动进程。

## Response

`response` 是叶节点，只产生一次返回：

```json
{
  "type": "response",
  "expect": {
    "protocol": "openai",
    "stream": true,
    "model": "local-maas-test"
  },
  "response": {
    "blocks": [
      {
        "type": "reasoning",
        "chunks": ["first ", "second"]
      },
      {
        "type": "message",
        "text": "calling bash"
      },
      {
        "type": "tool_call",
        "name": "bash",
        "arguments": {
          "command": "printf ok"
        }
      }
    ],
    "usage": {
      "input_tokens": 10,
      "output_tokens": 5
    }
  }
}
```

`expect` 可省略。指定后，protocol、stream 或 model 不匹配返回 409，并把当前 response 保存在 pending 中；后续匹配请求仍会取得同一 response。

reasoning 和 message 必须提供 `text` 或非空 `chunks`，且不能同时提供。tool call id 由协议 adapter 根据 response index 和 block index 生成，模板不保存 wire id。

`response.stop` 可为：

- `complete`
- `tool_call`
- `length`

省略时根据是否存在 tool call 自动推导。

## Sequential

```json
{
  "type": "sequential",
  "generators": [
    {
      "type": "response",
      "response": {
        "blocks": [{"type": "message", "text": "A"}]
      }
    },
    {
      "type": "response",
      "response": {
        "blocks": [{"type": "message", "text": "B"}]
      }
    }
  ]
}
```

它按顺序产生 `A, B`，随后 exhausted。无限 generator 只能是最后一个子节点。

## Loop

有限 loop：

```json
{
  "type": "loop",
  "count": 2,
  "generator": {
    "type": "response",
    "response": {
      "blocks": [{"type": "message", "text": "A"}]
    }
  }
}
```

省略 `count` 表示永久循环。显式 `count: null` 是配置错误。loop body 必须有限，保证每次 iteration 能结束。

## Random

```json
{
  "type": "random",
  "count": 4,
  "seed": 7,
  "generators": [
    {
      "type": "response",
      "response": {
        "blocks": [{"type": "message", "text": "A"}]
      }
    },
    {
      "type": "response",
      "response": {
        "blocks": [{"type": "message", "text": "B"}]
      }
    }
  ]
}
```

每个 iteration 均匀选择并完整运行一个子 generator。省略 `count` 表示永久选择；所有子 generator 必须有限。

节点 `seed` 优先于 CLI `--random-seed`。同一模板和 seed 在每次全新启动服务时产生相同序列。

## Usage

模板里的 `usage.input_tokens` 是本次 emission 的增量；runtime 将其加入当前服务进程的累计值。`output_tokens` 只表示当前 emission。

若永久 response 每次配置：

```json
{"input_tokens":128,"output_tokens":16}
```

前三次返回的 input tokens 是 `128, 256, 384`，output tokens 始终是 `16`。

## 启动期约束

- 模板必须是 UTF-8 JSON object；
- 根 generator 必须包含非空的英文 `description`；
- 未知字段直接失败；
- generator 深度、节点数和模板字节数受 CLI 配置限制；
- sequential、loop 和 random 都必须包含实际可产生 response 的子树；
- 不可到达节点、无限 loop body 和无限 random child 直接失败；
- 所有配置错误都在监听端口前报告。
