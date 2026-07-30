# Local MaaS Server

这是一个基于 Python 3 标准库的本地 MaaS 测试服务。启动时选择一个 JSON 剧本模板，服务按顺序取得协议无关的 LLM 返回内容，再转换成客户端请求的真实 MaaS 协议。HTTPS 默认使用本机 OpenSSL 和临时端口 best-effort 启动。

当前支持：

- OpenAI-compatible Chat Completions；
- Anthropic Messages；
- HTTP 和临时证书 HTTPS；
- direct JSON 和 SSE；
- response、sequential、loop、random、action_pool generator；
- 可复用 action pools 和请求工具 schema 驱动的 alias 转换；
- 根据每次外部请求的大小计算 input token usage；
- 可配置 TTFT/TPOT 返回节奏。

## 快速启动

```bash
python3 tests/v2/common/test-suites/local-maas-server/server.py \
  --scenario alternating-message-loop \
  --http-bind-port 42117
```

启动成功后会按 listener 输出实际监听地址、服务类型和 REST API。默认 HTTPS 成功时还会输出本次运行的临时端口和 CA；如果 OpenSSL 不可用，服务打印 warning 并只保留 HTTP。显式传入任一 HTTPS/TLS 参数时，HTTPS 启动失败会直接终止启动。

```text
Local MaaS server is ready

Listener 1
  service:    HTTP
  listen:     127.0.0.1:42117
  origin:     http://127.0.0.1:42117

  REST APIs:
    OpenAI-compatible
      POST  /v1/chat/completions
    Anthropic
      POST  /v1/messages

Listener 2
  service:    HTTPS
  listen:     127.0.0.1:45821
  origin:     https://127.0.0.1:45821
  ca bundle:  /tmp/local-maas-tls-.../combined-ca.pem

Press Ctrl+C to stop.

Please run with the Local MaaS CA: SSL_CERT_FILE=/tmp/local-maas-tls-.../combined-ca.pem <command>
```

使用 `--disable-https` 可明确只启动 HTTP。

默认不校验 API key。客户端自身要求非空 key 时可使用任意测试值。需要同时验证服务端鉴权时：

```bash
LOCAL_MAAS_API_KEY=local-test-key \
python3 tests/v2/common/test-suites/local-maas-server/server.py \
  --scenario alternating-message-loop \
  --http-bind-port 42117 \
  --api-key-env LOCAL_MAAS_API_KEY
```

## 请求

OpenAI direct：

```bash
curl http://127.0.0.1:42117/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"local-maas-test","messages":[],"stream":false}'
```

OpenAI SSE：

```bash
curl -N http://127.0.0.1:42117/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"local-maas-test","messages":[],"stream":true,"stream_options":{"include_usage":true}}'
```

Anthropic direct：

```bash
curl http://127.0.0.1:42117/v1/messages \
  -H 'Content-Type: application/json' \
  -d '{"model":"local-maas-test","messages":[],"max_tokens":128,"stream":false}'
```

Anthropic SSE 把同一请求中的 `stream` 改为 `true`。

HTTPS 请求使用启动信息返回的临时 CA bundle：

```bash
curl --cacert /tmp/local-maas-tls-.../combined-ca.pem \
  https://127.0.0.1:42118/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"local-maas-test","messages":[],"stream":false}'
```

## 剧本播放

一次服务启动只创建一个 lazy iterator。每个匹配 expectation 的 MaaS 请求消费下一份 response。input tokens 根据当前外部请求的规范化 JSON 字符数计算，不从剧本读取，也不跨请求累计；Agent 携带的历史越长，本次 usage 就越大。有限剧本耗尽后返回协议对应的 409 错误。

请求声明的工具及 schema 会先经 Alias Converter 反向转换为本次 `GenerationOptions`。运行时先统一工具名和参数名的大小写，再由 alias 表处理 `read_file -> read` 等语义别名。random 和 action_pool 只从可正向转换的候选中生成；没有声明工具时，混合 pool 只会生成 reasoning/message action。

服务不会要求真实 MaaS 协议之外的 header 或 query 参数。不同测试需要独立剧本状态时，分别启动服务进程；重启服务即从剧本开头重新播放。

一个服务进程面向一个串行 Agent 或测试。并行 case 必须各自启动服务进程，不能共享同一个 iterator。请求匹配 expectation 并取得 response 后，该 response 即视为已消费；客户端断连或发送失败不会回滚剧本。

## 内置模板

模板位于 `scenario/scenario_generator/templates/`：

- `finite-sequence.json`
- `finite-middle-loop.json`
- `alternating-message-loop.json`
- `random-message.json`
- `bash-tool-roundtrip.json`
- `bash-home-loop.json`
- `action-pools/random-light-exec.json`
- `action-pools/adaptive-tool-or-message-loop.json`
- `action-pools/random-file-operation.json`
- `action-pools/reasoning-length-loop.json`
- `action-pools/short-reasoning-sequential-cycle.json`
- `action-pools/long-reasoning-short-operation.json`

可复用 action 位于 `scenario/scenario_generator/action_pools/`，按 reasoning 长度、工具种类和操作重量分池。模板只选择和组合 pool，不复制具体动作。通过 `--templates-dir` 和 `--action-pools-dir` 使用外部目录。

## 配置

配置按模块分层：

```text
LocalMaaSConfig
├── generator
├── tool_alias
├── protocol
├── schedule
└── server
    ├── http
    └── https
```

`LocalMaaSServer.start()` 非阻塞返回当前 status。运行期间调用 `reset()` 会把 scenario iterator、pending response 和 response index 重置到初始状态，同时保持 listener、端口和临时证书不变；`stop()` 负责关闭服务。

运行下面的命令查看全部入口：

```bash
python3 tests/v2/common/test-suites/local-maas-server/server.py --help
```

所有容量、超时、随机 seed、TTFT 和 TPOT 都有 CLI 配置；非法配置会让启动失败。单次请求错误只影响当前请求。

## 源码入口

```text
server.py                         进程启动入口
config.py                         顶层配置聚合
scenario/                         剧本模型、播放状态和 generator
protocol/                         协议 adapter 与 registry
schedule/                         TTFT/TPOT 节奏控制
server_core/server.py             非阻塞 LocalMaaSServer 生命周期
server_core/                      application、endpoint 和 HTTP/HTTPS connection
utils/                            JSON、日志和进程退出信号等待
```

进一步说明：

- [目标架构](docs/architecture.md)
- [HTTPS](docs/https.md)
- [剧本格式](docs/scenario-format.md)
- [协议适配器](docs/protocol-adapters.md)
- [返回调度](docs/response-scheduling.md)
