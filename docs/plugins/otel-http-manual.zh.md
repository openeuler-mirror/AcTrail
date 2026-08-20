# otel-http 插件启用手册

内置 OTLP/HTTP 实时出境插件（`otel-http`）的启用步骤。插件语义、配置字段和安全
约束见 [`examples/plugins/builtin/otel-http/README.zh.md`](../../examples/plugins/builtin/otel-http/README.zh.md)，
完整回归验证流程见 [`tests/v2/regression/otel_http/README.zh.md`](../../tests/v2/regression/otel_http/README.zh.md)。

## 先决条件

`otel-http` 的执行代码是 builtin，已编译进 `actraild`，没有独立 artifact，加载也
不需要 `--grant`。`scripts/install-release.sh` 会把三个描述文件复制到
`${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}/otel-http/`：

```
otel-http.plugin.toml            # manifest
otel-http.config.toml            # 默认配置模板（带占位 Collector 地址）
otel-http.config.v1.schema.json  # 配置 schema
```

**安装不等于启用**：装完之后它只是可发现的候选包，daemon 不会创建 exporter，必须
显式加载。没跑过安装脚本时，直接引用源码路径
`examples/plugins/builtin/otel-http/` 同样可以加载。

配置里的 `[[headers]]` 需要 2026-08-06 之后构建的 `actraild`。

## 步骤 1：准备插件配置

复制一份模板到部署路径再改，不要直接改仓库里的模板：

```bash
sudo mkdir -p /etc/actrail/plugins/otel-http
sudo cp <repo>/examples/plugins/builtin/otel-http/otel-http.config.toml \
        /etc/actrail/plugins/otel-http/
sudo vi /etc/actrail/plugins/otel-http/otel-http.config.toml
```

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `endpoint` | `http://COLLECTOR_HOST:4318/v1/traces` | 占位地址，**必须**换成真实 Collector |
| `allow_insecure` | `true` | 明文 endpoint 必须显式为 `true`；`https://` 时设为 `false` |
| `attribute_mode` | `metadata-only` | 只发结构化元数据，不发命令行和 HTTP/LLM 内容；`llm.request` 仍保留 trajectory ID 与推断版本。`full` 只导出 daemon 已生成的动作属性，不会自动开启可选内容，且仅适用于可信 Collector 与链路 |
| `[action_kinds]` | `default = false` | 未列出的类型一律不发。模板已预开 `process.exec/exit`、`agent.*`、`llm.*`（含 `llm.tool_call/result`）、`mcp.*`、`enforcement.decision`、`command.invocation`；`file.*`、`fs.enumerate`、`http.message`、`sse.*` 默认关闭 |
| `tls_client_cert_path` / `tls_client_key_path` | 注释 | mTLS 两者必须同时配置；为明文 endpoint 配置 TLS 文件会被拒绝 |

### LLM 请求正文的三重授权

在 `[action_kinds]` 已允许 `llm.request` 的前提下，请求正文出境还要求三个条件同时
成立：

```toml
# daemon operator config
[semantic_retention.l0_llm_call]
request_content = "canonical_blocks"
request_body_export = "canonical_json"
# 可选；正文超过该规范化 UTF-8 字节上限时不发送，只标记 too_large
request_body_export_max_bytes = 131072

# otel-http plugin config
attribute_mode = "full"
```

`request_content` 允许 daemon 保存可重建正文，`request_body_export` 才让它在动作属性中
产生完整正文副本，`attribute_mode` 决定这些已产生的属性能否离开 daemon。把三层分开
是为了让已有 `full` 部署升级后仍保持原出境范围；否则升级本身就会开始外送完整对话、
工具结果和 agent 读取的文件内容。启用前应确认额外本地副本符合留存政策，Collector
和传输链路能承载敏感内容，并按接收端上限设置 `request_body_export_max_bytes`。

### 工具调用、结果与 subagent 关系

新版本把模型原生工具交互投影为 `llm.tool_call` / `llm.tool_result`，把配置的逻辑
Agent 工具投影为 `agent.invocation`，并通过 OTLP Span Links 保留调用、结果、trajectory
和子请求关系。旧配置采用显式白名单，升级后必须补上：

```toml
[action_kinds]
"llm.tool_call" = true
"llm.tool_result" = true
"agent.invocation" = true
```

工具结果默认只生成 ID、错误态、规范化字节数、哈希和绑定状态，不生成正文；这些元数据
也只有 `attribute_mode = "full"` 时才会出境。需要正文时还要显式授权 daemon 生成正文属性：

```toml
# daemon operator config
[semantic_retention.l0_llm_call]
tool_result_content_export = "canonical_json"
tool_result_content_export_max_bytes = 131072

# otel-http plugin config（元数据与正文均需此项）
attribute_mode = "full"
```

`agent_invocation.tool_names` 默认识别 `Agent`、`Task`、`task` 和 `spawn_agent`；不同
Agent runtime 使用其他逻辑工具名时应在 operator config 中显式补充。子请求关联只接受
唯一 prompt 指纹命中，不使用时间邻近猜测。

## 步骤 2：加载

daemon 必须已在运行，control socket 要求 host-root peer，因此需要 `sudo`：

```bash
sudo <repo>/target/release/actraild --config /etc/actrail/actraild.conf plugin load \
  --manifest <repo>/examples/plugins/builtin/otel-http/otel-http.plugin.toml \
  --plugin-config /etc/actrail/plugins/otel-http/otel-http.config.toml \
  --instance my.otel-http
```

预期输出 `loaded instance=my.otel-http` 和 `warnings=none`。`--manifest` 也可以
指向安装目录下的 `~/.actrail/plugins/otel-http/otel-http.plugin.toml`。

部署形态固定加载时改用 `operator.conf` 的启动清单，daemon 启动即生效：

```toml
[plugins.startup]
enabled = true
failure_policy = "fail-fast"

[[plugins.startup.load]]
instance = "my.otel-http"
manifest = "/usr/share/actrail/plugins/otel-http/otel-http.plugin.toml"
plugin_config = "/etc/actrail/plugins/otel-http/otel-http.config.toml"
host_grants = []
```

需要在 Web 页面点选加载时，把 `[plugins.discovery] directory` 指向插件安装根目录
（或仓库的 `examples/plugins/builtin`），重启 `actrailweb` 后候选即可见。

## 步骤 3：验收（不能省）

```bash
sudo <repo>/target/release/actrailctl --config /etc/actrail/actraild.conf \
  launch --name otel-http-smoke -- /bin/true

sudo <repo>/target/release/actraild --config /etc/actrail/actraild.conf \
  plugin status --instance my.otel-http
```

**`endpoint` 配错不会让加载失败**：插件仍是 `active`，`plugin load` 仍返回 0，只有
`plugin status` 会体现投递失败。投递成功时 `metric.otel_http.successful_batches`
增长且 `last_error=none`；Collector 不可达时表现为：

```
observed_records=8
dropped_records=3
last_error=connect 127.0.0.1:4318: Connection refused (os error 111)
metric.otel_http.retry_attempts=2
metric.otel_http.dropped_batches=1
metric.otel_http.successful_batches=0
```

`observed_records` 增长说明采集侧到插件的数据流已经打通，问题只在出境链路。

卸载：

```bash
sudo <repo>/target/release/actraild --config /etc/actrail/actraild.conf \
  plugin unload --instance my.otel-http
```

卸载会触发 consumer 的 `finish`，未发送的尾批次按 `shutdown_flush_deadline_ms`
尝试发出；daemon 不重启。

## 排错

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| `plugin load` 权限被拒 | control socket 要求 host-root peer | 使用 `sudo` |
| `otel-http plaintext endpoint requires allow_insecure = true` | 明文 endpoint 未显式确认 | 设 `allow_insecure = true`，或改用 `https://` |
| `otel-http action_kinds.default must be false` | 试图用 `default = true` 放行全部类型 | 出境边界必须显式白名单，逐项开启 |
| `otel-http TLS options require an https:// endpoint` | 为明文 endpoint 配置了 TLS 文件 | 去掉 TLS 路径或改用 `https://` |
| 状态 `active` 但 Collector 收不到 | endpoint 不可达、证书不匹配或类型未放行 | 看 `plugin status` 的 `last_error`、`dropped_records`、`observed_records` |
| `observed_records` 一直为 0 | 该 trace 产生的 action 类型全被 `[action_kinds]` 关闭 | 按需打开对应类型 |
| 加载带 `[[headers]]` 的配置失败 | 二进制早于 2026-08-06 | 重新构建 `actraild` |

没有现成 Collector 时，`tests/v2/regression/otel_http/` 下带有一个本地 OTLP/HTTP
JSON receiver（`receiver.py`），可按该目录 README 的步骤 3 启动，验证流程可以完全
离线完成。
