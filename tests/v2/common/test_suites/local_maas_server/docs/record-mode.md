# 录制模式（Record Mode）

录制模式把 local_maas_server 变成一个**转发 + 录制**代理：Agent 以本 server
为 LLM 目标发起真实任务，服务端把请求转发给真实的上游 MaaS，把响应流式回传，
同时按会话把"模型的真实返回序列"落盘；任务结束后收束成一份可被普通播放模式
直接回放的 recorded 剧本。

## 边界

- 上游 MaaS 默认只支持 **OpenAI-compatible** 协议（`/v1/chat/completions`）。
  Anthropic 路径的请求在录制模式直接返回 `501 unsupported_upstream_protocol`。
- 一个 API KEY 对应一个录制会话；无 key / 错误 key 的 MaaS 请求直接 `401` 拒绝。
- 会话状态只保存在内存中；cache 和收束后的剧本文件落盘到 `--recordings-dir`。
- 录制只保存完整的模型返回序列（reasoning / message / tool_call / stop /
  output_tokens），时间戳、请求/响应 id、错误等易变内容不落盘；
  SSE 断流或未完整结束的响应**不记录**。
- 一次会话面向一个串行 Agent；并行场景各自创建会话。

## 启动

```bash
python3 tests/v2/common/test_suites/local_maas_server/server.py \
  record \
  --http-bind-port 42117 \
  --recordings-dir /tmp/maas-recordings
```

录制模式不需要 `--scenario`；subcommand 已按模式划分参数，record 不注册
replay/transport 的参数。

## REST API

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `POST` | `/record/sessions` | 创建录制会话，返回 `session_id` 和 `api_key` |
| `GET` | `/record/sessions` | 列出会话 |
| `POST` | `/record/sessions/{session_id}/finalize` | 收束：关闭录制并生成 recorded 剧本 |

创建会话请求体：

```json
{
  "tools": ["run_command", "read_file"],
  "upstream": {
    "base_url": "http://127.0.0.1:8000",
    "api_key": "upstream-secret",
    "model": "optional-model-override"
  }
}
```

`tools` 是纯工具名白名单（大小写不敏感）。收束请求体可选 `scenario_id`
（默认 `recorded-<session_id>`）。

`upstream` 可以省略。省略时按 transport 模式同款顺序自动解析：
`LOCAL_MAAS_UPSTREAM_URL` / `LOCAL_MAAS_UPSTREAM_API_KEY` /
`LOCAL_MAAS_PROTOCOL`（可选 `LOCAL_MAAS_UPSTREAM_MODEL`）→
`DEEPSEEK_API_KEY`（探测 `https://api.deepseek.com/models` 并取第一个
model）。全部缺失时创建会话返回 `400 invalid_session`。

## 请求处理链

```text
Agent --Authorization: Bearer <session api_key>--> local_maas_server
    -> 会话 API key 校验（无/错 -> 401）
    -> ToolPruner：tools 白名单剪枝 + 可选 model 覆盖
    -> OpenAIUpstreamClient：转发到真实上游（direct 或 SSE 逐行）
    -> 响应原样流式回传 Agent
    -> ResponseParser：direct/SSE 解析并标准化
    -> 完整响应追加一行到 <session_id>.cache.jsonl
```

## cache 格式（`.cache.jsonl`）

每行一个完整响应，字段即 recorded 剧本所需内容：

```json
{
  "protocol": "openai",
  "stream": false,
  "model": "m",
  "blocks": [
    {"type": "reasoning", "chunks": ["..."]},
    {"type": "message", "text": "..."},
    {"type": "tool_call", "name": "bash", "arguments": {"command": "..."}}
  ],
  "stop": "tool_call",
  "output_tokens": 18
}
```

## 工具名/参数名标准化

录制和回放共享同一个标准中间态：`scenario/tool_alias/schema.py` 的
`ToolSchemaRegistry`。每个工具是一个 `ToolSchema`（规范名 + 别名 + 字段），
每个字段是一个 `ToolField`（规范 key + 别名）；名称匹配先做机械归一化
（`utils/naming.py`，大小写与 `filePath`/`file_path`/`filepath` 这类分隔符差异
自动等价），再查显式别名。

- 录制时 `canonicalize_call` 把任意 agent 的工具调用转成规范形态，例如
  `run_command(cmd)`、`READ(file_path)` 都落盘为 `bash(command)` /
  `read(path)`；
- 回放时 `convert_call` 把规范调用反查成客户端声明的工具名/参数名，
  校验 required 与值类型；
- 没见过的工具自动注册一个"同名字、无别名"的严格匹配 schema 继续录制/回放，
  未知客户端工具被忽略；回放遇到客户端无法执行的 tool 轮次会跳过取下一个；
- 旧剧本在加载时通过同一套 `canonicalize_template` 自动转成中间态。

支持新 agent 只需在 `scenario/tool_alias/config.py` 的默认 `ToolSchema`
列表里增加一条声明（工具名/字段 key + 别名）；完全未知的工具连声明都不需要。

## 收束与回放

```bash
curl -X POST http://127.0.0.1:42117/record/sessions/<session_id>/finalize \
  -H 'Content-Type: application/json' \
  -d '{"scenario_id": "recorded-perf-run"}'
```

成功后生成三个文件（默认剧本 templates 目录的子目录，已通过
`ScenarioLoader` 校验），并删除该会话的 `<session_id>.cache.jsonl`：

- `<id>.meta.json`：元信息（description、`type=recorded`、轮次/工具统计、
  `tool_source` / `message_source` 引用）；
- `<id>.tool.jsonl`：含 `tool_call` 的轮次，一行一轮；
- `<id>.message.jsonl`：不含 `tool_call` 的轮次（最终回答），一行一轮。

剧本按是否含 `tool_call` 拆成两个队列，回放时从对应 jsonl 按需逐行读取，
不把整个序列载入内存。

回放时请求带 `tools` 从 `tool` 队列取下一轮，请求不带 `tools` 从
`message` 队列取下一轮，解决 xiaoo 在循环中发出"无工具请求"时被严格
工具校验 409 的问题。`tool` 队列耗尽后，带 `tools` 的请求回退到
`message` 队列继续取下一轮，保证始终声明工具的 agent（如 opencode）
也能拿到录制的最终回答而不是提前 409；`message` 队列耗尽后默认从头循环
（`--loop-exhausted-messages`，可用 `--no-loop-exhausted-messages` 关闭），
因此被无工具请求（如 opencode 的会话标题生成）提前消费的最终回答仍能
再次提供给结尾请求。回放直接用默认 `--templates-dir`，scenario id 带
`recorded/` 前缀：

```bash
python3 tests/v2/common/test_suites/local_maas_server/server.py \
  replay \
  --scenario recorded/recorded-perf-run-<时间戳>-<hash5> \
  --http-bind-port 42118
```

下次启动缺省 `--scenario` 时打印的可用剧本列表（helper）会自动包含
`recorded/...` 条目。

回放语义与普通剧本一致：按序消费、耗尽返回 409、`--ttft-milliseconds` /
`--tpot-milliseconds` 控制节奏（性能对比场景）。

## 端到端验证

`test/run_record_e2e.py` 覆盖：会话创建、无 key 拒绝、白名单剪枝、
direct/SSE 转发、cache 标准化、收束校验、回放一致性，以及有凭据时的
真实上游探测与请求轮。

## 录制真实 Agent 剧本

`test/run_real_agent_record.py` 用真实 Agent（xiaoo，OpenAI 兼容）录制
一个长任务：

```bash
python3 test/run_real_agent_record.py \
  --agent xiaoo \
  --prompt "只读分析 ..." \
  --max-turns 60 \
  --name recorded-xiaoo
```

也支持录制 opencode 会话（`--agent opencode`，工具白名单默认
`read,glob,grep`）：

```bash
python3 test/run_real_agent_record.py \
  --agent opencode \
  --prompt "只读分析 ..." \
  --name recorded-opencode
```

- `--prompt` 与 `--prompt-file <file>` 二选一（必填）；
- `--tools` 默认 `file_read,glob,grep`；
- 需要 `DEEPSEEK_API_KEY` 作为上游凭据；
- 收束产物按新命名规范落到 `templates/recorded/`，脚本结尾打印
  `scenario id`、轮数、block/tool 统计与剧本文件路径。
