# HTTP 与 LLM 消息来源验收

本测例启动真实 Xiaoo，通过 `file_read` 读取临时目录中的随机内容，再要求模型回答该内容。每次执行至少包含两个成功模型轮次，并检查实际 provider 返回的 HTTP 404 重试。

## 运行

准备包含最新实现的 release `actraild`、`actrailctl`、`actrailviewer`、`actrailweb`，并配置可正常调用模型的 Xiaoo。使用已有真实 HTTP 服务作为普通 GET 的目标：

```bash
sudo -E python3 tests/v2/regression/http_llm_sources/run_e2e.py \
  --xiaoo /home/yzh/.cargo/bin/xiaoo \
  --xiaoo-config /home/yzh/.config/xiaoo/config.toml \
  --plain-url http://127.0.0.1:8317/
```

`--plain-url` 必须指向实际服务。测试不启动模型模拟服务，不注入编造的 HTTP 响应。若当前 provider 不产生 404，结果报告该路径未覆盖，不能算作完整通过。

## 隔离与证据

每次运行新建 `actrail-http-llm-sources-*` 目录，通过 `actrailctl init -f --patch` 刷新默认配置。控制 socket、PID、数据库、日志、TLS socket、资源数据库和导出目录都指向该目录。全部控制、launch 和 viewer 命令显式携带隔离配置。

测试依次运行两个独立 daemon 配置：默认 LLM 投影开启，以及仅关闭 `semantic_retention.l0_llm_call.enabled`。后者仍使用真实 Xiaoo，并要求 HTTP 请求—响应事实存在而 LLM actions 为零。两个阶段各额外执行一次真实 curl HTTP/1 GET，检查普通 HTTP 的独立可见性。

结果保存在打印的证据目录，目录权限 700、文件权限 600。`report.json` 包含通过状态、trace ID、调用计数和失败原因；各阶段保留 agent 日志、viewer action/link JSON 和只读 web 日志。action JSON 可能包含请求头和正文，应按敏感观测证据保存。成功和失败均停止测试 daemon 与只读 web，保留数据供检查。

## 验收约束

- 来源 action 的协议必须实际为 HTTP/1；HTTP/2 或 WebSocket 不能充当该项覆盖。
- 每个 LLM request/response 有唯一 HTTP message 来源，每个 request/response 有唯一 Call owner。
- Call 的 request/response 属性与 link 一致，HTTP 请求—响应配对与 LLM 配对一致，trace、process 和 connection stream identity 一致。
- 每个已关联 LLM 请求的失败 HTTP 响应生成同状态码的失败 LLM response。
- 正常工作负载的 request/response 完整结束，终态时间严格早于 trace 关闭，且无 trace-close finalization 标记。
- 正常回答包含随机 marker；canonical request API 返回完整正文，后续工具结果上下文中包含 marker。
- 普通 HTTP、禁用 LLM 时的 HTTP 观测和请求—响应链接保留。

## 参数

`--runs` 默认 2，控制 LLM 开启阶段的 Xiaoo 执行次数；关闭阶段固定执行一次。`--max-turns` 默认 4，`--timeout` 默认 180 秒，`--min-http-errors` 默认 1，要求实际观察至少一个 HTTP 404。`--content-max-bytes` 默认 8 MiB，限制每次 canonical 内容读取。`--output-parent` 默认系统临时目录，`--bin-dir` 默认仓库 `target/release`。`ACTRAIL_HTTP_SOURCES_WEB_PORT` 可指定只读 web 端口，否则分配可达的 loopback 端口。

## HTTP/2 真实协议覆盖

`run_protocol_e2e.py` 使用真实 Claude 或 Codex 读取隔离目录的随机文件，执行 LLM 开启和关闭两个阶段。它要求持久化来源实际为 HTTP/2，并检查相同的 HTTP/LLM 配对、工具轮次、正文与终态约束。当前 HTTP/2 header 未解码，状态码未知，因此该入口不要求 HTTP 404。普通 HTTP/1 Xiaoo 验收入口仍负责错误与重试覆盖。

```bash
python3 tests/v2/regression/http_llm_sources/run_protocol_e2e.py --agent claude
```

`--agent-binary` 可指定实际二进制，`--model` 可指定已配置可用模型；默认使用 agent 自身配置。`--timeout` 默认 240 秒。实际服务或客户端回落到 HTTP/1、WebSocket，或捕获中没有 HTTP/2 来源时，验收失败并保留证据，不能视为 HTTP/2 通过。

`--protocol http1` 可将同一真实 agent 工具流程用于 HTTP/1 来源验收；默认是 `http2`。两种结果独立记录，不能用 HTTP/1 的通过代替 HTTP/2 覆盖。

Claude/Codex 入口记录实际连接复用数量，允许客户端为不同轮次创建独立连接。Xiaoo 专用入口仍要求同连接复用，并覆盖同连接重试后成功。

Codex 的 HTTP 验收模式用单次命令参数选择 `openai-http` provider，保留 OpenAI 登录认证并设置 `supports_websockets=false`，要求真实 HTTP/SSE 调用；同时关闭 `features.enable_request_compression`，让请求正文处于现有解析能力范围。测试不修改全局配置。登录模式仍决定实际服务端地址。HTTP/2 是否被协商始终由捕获的协议事实判断。

## WebSocket 真实来源覆盖

```bash
python3 tests/v2/regression/http_llm_sources/run_protocol_e2e.py --agent codex --protocol websocket
```

该模式保留 Codex 默认 provider 的 WebSocket 能力，使用真实 Responses 会话执行相同工具任务。每个请求必须直接关联一条 `websocket.message`，响应关联实际消费的消息；所有来源、请求和响应必须属于同一进程与连接。来源消息必须完整结束，LLM 终态早于 trace 关闭，回答与后续请求正文包含实际文件内容。测试拒绝 WebSocket LLM action 上的 HTTP 来源链接。LLM 关闭阶段要求原生 WebSocket 消息仍可观测。

客户端回落 HTTP 时，WebSocket 验收失败并保留证据。该结果不能充当 WebSocket 覆盖。每次运行使用刷新默认配置和独立数据目录。

可通过 `--websocket-relay-upstream https://chatgpt.com/backend-api/codex` 使用测试专用 loopback TLS 字节中继：Codex 向本地发送明文 WebSocket，中继验证上游 TLS 证书并转发到真实服务，只调整初始 HTTP Host。模型响应和工具流程均由真实服务与 Agent 执行。该方式用于独立验证 socket 身份上的 WS 消息、语义与留存，不证明 Rustls 指针在对象移动后的身份问题已解决。报告明确记录中继覆盖与上游地址，测试不记录认证头或转发正文。

`--retain-transport` 仅用于失败诊断：在新默认配置上开启 L4 stats/正文留存，并使用 `configured_layers` 保留已被语义消费的原文。数据库可能包含认证头和请求正文，应保持隔离目录权限，诊断后按本地证据保留策略处理。正常验收省略该参数。
