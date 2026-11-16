# Scenario Generator 格式

每个 scenario 由两个部分构成：

- `<id>.meta.json`：元信息（英文 `description`、generator `type`、`infinite`、
  轮次与工具统计、序列文件引用）。registry 只扫描 meta，列表/选择不加载序列；
- 序列文件：generator 类剧本是单个 `<id>.seq.json`（generator 对象，不再含
  `description`）；recorded 剧本按是否含工具调用拆成 `<id>.tool.jsonl` 与
  `<id>.message.jsonl`，一行一轮、按需读取。

文件相对路径是 scenario id；没有额外的 `version`、`name` 或顶层 `generator` 包装。例如：

```json
{
  "name": "finite-sequence",
  "description": "Returns two assistant messages in order and then exhausts.",
  "type": "sequential",
  "infinite": false,
  "sequence": "finite-sequence.seq.json",
  "rounds": 2,
  "tool_rounds": 0,
  "message_rounds": 2,
  "tools": []
}
```

recorded 与非 recorded 的 meta 使用同一组统计字段；无限轮次用 `null` 表示
（展示为 `inf`），例如纯消息无限循环为 `rounds: null, tool_rounds: 0,
message_rounds: null`。统计由 generator 的有界 dry-run 得到（全工具接受）。

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
      "output_tokens": 5
    }
  }
}
```

`expect` 可省略。指定后，protocol、stream 或 model 不匹配返回 409，并把当前 response 保存在 pending 中；后续匹配请求仍会取得同一 response。

reasoning 和 message 必须提供 `text` 或非空 `chunks`，且不能同时提供。tool call 使用剧本的规范 `name` 和参数；它不感知客户端工具别名。tool call id 由协议 adapter 根据 response index 和 block index 生成，模板不保存 wire id。

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

## Action Pool

`action_pool` 从 `--action-pools-dir` 下的一个或多个相对目录选择 action：

```json
{
  "type": "action_pool",
  "pools": [
    "tool/exec/light",
    "tool/file/read"
  ],
  "selection": "random",
  "count": 2,
  "seed": 7
}
```

每个 pool 会递归发现 `.json` 文件。重叠 pool 选中的同一文件只保留一次。每个 action 文件：

- 根节点必须包含英文 `description`；
- 删除 `description` 后必须是合法且有限的 generator；
- 可以由 response、sequential、loop 或 random 组成；
- 不能再次引用 action_pool。

`selection` 可为 `random` 或 `sequential`，默认 `random`。`count` 表示选择并完整运行 action 的次数；省略表示永久选择。所有目录发现、JSON 读取和 generator 校验都在启动监听端口前完成，运行期不访问文件系统。

每次选择前，action_pool 使用当前请求的 `GenerationOptions` 过滤候选。包含无法映射工具调用的 action 不参加本次 random 或 sequential 选择，因此 generator 不会先选中非法 action 再依赖协议层兜底。

## Tool Alias 转换层

action 保持普通工具调用 contract：

```json
{
  "type": "tool_call",
  "name": "bash",
  "arguments": {
    "command": "pwd"
  }
}
```

generator 和所有剧本都只认识这套规范调用，不引用 alias 配置或具体客户端。

OpenAI 和 Anthropic adapter 从当前请求提取实际工具名称及 input schema。抽象 `ToolAliasConverter` 先反向生成本次请求的 `GenerationOptions`，generator 据此筛选可以转换的 action。scenario 取得规范 response 后、协议编码前，同一个 converter 再执行正向转换。

`SchemaToolAliasConverter` 使用两层映射：

1. 每次请求创建运行时映射，对实际工具名和参数名执行 `casefold()`，但保留原始名称供 wire response 使用。例如 `READ`、`Read` 和 `ReAd` 都得到 `read`。
2. `ToolAliasConfig` 只记录规范化名称到剧本标准名的语义 alias。例如 `read_file -> read`、`terminal -> bash`；大小写变体不进入 alias 表。

因此 `Read(file_path)`、`read_file(path)` 和 `READ(PATH)` 都可以匹配剧本的 `read(path)`，返回时仍使用请求实际声明的工具名和参数名。

请求没有声明 tools 时，GenerationOptions 不包含任何工具候选。混合 action_pool 只会选择纯 reasoning/message action；只有工具 action 的 pool 则返回 409，但不会推进选择次数或耗尽 generator。

新增客户端命名或 schema 支持时，只扩展 `ToolAliasConfig` 或新增 converter 实现并交给 factory 构造，不修改 scenario 和 action pool 文件。

若请求没有提供兼容工具，或者 schema 需要 action 无法提供的必填参数，本次请求返回 409 scenario mismatch。当前 response 保持 pending，不会返回未声明的工具，也不会推进剧本。

## Usage

剧本只配置返回侧的 `usage.output_tokens`；reasoning、message 和 tool call 内容也都由 response blocks 控制。`input_tokens` 不属于剧本，其值由 protocol adapter 根据每次外部请求的规范化 JSON 字符数计算，每个字符视为一个 token。

例如 response 可以配置：

```json
{"output_tokens":16}
```

同一个 response 被循环使用时，output tokens 始终是 `16`；input tokens 则随每次请求实际携带的消息、工具定义和历史变化，不跨请求累计。

## 启动期约束

- 模板必须是 UTF-8 JSON object；
- 根 generator 必须包含非空的英文 `description`；
- 未知字段直接失败；
- generator 深度、节点数和模板字节数受 CLI 配置限制；
- sequential、loop、random 和 action_pool 都必须包含实际可产生 response 的子树；
- 不可到达节点、无限 loop body 和无限 random child 直接失败；
- 所有配置错误都在监听端口前报告。
