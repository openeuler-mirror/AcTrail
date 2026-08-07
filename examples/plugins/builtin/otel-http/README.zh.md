# 内置 OTLP/HTTP 观测插件

类别：内置观测消费者；状态：通用候选能力。

该插件把语义 action 通过 OTLP/HTTP 实时发送到 Collector，宿主机、容器或虚拟机
部署都可以使用。发送队列、批次、
连接/请求超时、重试次数和 shutdown flush 都有明确上限；队列满或重试耗尽时记录
drop，但不提供 WAL、at-least-once 或可靠投递保证。

出境边界采用显式授权：`[action_kinds]` 决定允许发送的 action 类型，默认
`attribute_mode = "metadata-only"`，只发送 action/trace 标识、类型、状态、时间、
进程标识等结构化元数据，并以 action 类型代替可能含内容的 span 标题；不发送命令行、
HTTP/LLM 内容等采集属性。只有 Collector
及传输链路均受信任且业务确有需要时，才应显式改成 `attribute_mode = "full"`。
插件只导出终态 action；同一 action 的 `in_progress` 修订不会形成重复 span。

目录包含：

- `otel-http.plugin.toml`：builtin observation-consumer manifest；
- `otel-http.config.toml`：部署配置，使用前必须替换 Collector 占位地址；
- `otel-http.config.v1.schema.json`：Web/部署工具可使用的配置 schema。

可选的 `[[headers]]` 数组为每次 POST 附加自定义请求头，供需要识别发送方身份的
Collector 使用（例如 Agent Insight 用 `x-witty-api-key` 决定 trace 归属哪个账号）：

```toml
[[headers]]
name = "x-witty-api-key"
value = "<用户 API Key>"
```

写成数组而非键值表，是为了与本仓库其他插件配置一致：每个配置对象都由具名字段
构成，Web 配置面板据此渲染，因此请求头可以直接在页面上增删改。

由 endpoint、编码和请求体推导出来的 `host`、`content-length`、`content-type`、
`content-encoding`、`connection`、`transfer-encoding` 属于传输层，配置这些会被拒绝；
头名必须是 RFC 7230 token，头值不得含 CR/LF；同一个头名（不区分大小写）只能出现
一次，重复配置会直接拒绝加载，而不是静默取其一。凭据在该配置文件中以明文保存，
因此携带凭据时必须使用 `https://`。

## Web 端配置

展开已加载实例，再展开 **Configuration**，即可在 **Extra request headers** 下
增删请求头条目；头值按凭据处理，输入框做遮码显示。与其他文件型插件一样，页面上
点 **Test** 校验通过后再点 **Update configuration**，daemon 会用新配置重新加载
导出器，立即生效。校验失败时保持原配置不变；保留头、非法头名和重复头名都会在
这一步返回明确错误。

页面提交的配置不写回 `otel-http.config.toml`，daemon 重启后仍以文件内容为准。
正式部署应直接维护配置文件，页面修改适合联调期临时调整。

生产环境应使用 `https://`。`http://` endpoint 只有在显式设置
`allow_insecure = true` 时才会被接受；mTLS 的客户端证书与私钥必须同时配置，为
plaintext endpoint 配置 TLS 文件会被拒绝。不同部署形态可以通过各自的 `operator.conf`
和插件发现目录加载该能力。

该插件只覆盖有界 best-effort 出境能力。关闭时，调用方最多等待
`shutdown_flush_deadline_ms`；截止时仍未被 Collector 确认的记录会按 drop 上报，
阻塞中的同步请求会脱离关闭调用并在返回后自行退出。因此 Collector 有可能已收到、
但客户端尚未来得及确认，接收端应按 trace/span ID 去重。真实 Collector 互操作、
证书分发与轮换、各部署形态的生命周期故障矩阵和可靠投递仍需独立验收。
