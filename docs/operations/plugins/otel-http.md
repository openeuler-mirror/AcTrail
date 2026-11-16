# 启用 otel-http

> 本文指导插件管理员将获准出境的观测 span 实时发送到指定 OTLP/HTTP Collector。

`otel-http` 是编译进 `actraild` 的内置异步观测插件，通过 OTLP/HTTP 实时发送 span。
插件包只提供 manifest、默认配置和 schema；安装后仍须显式加载。它不需要
host grant。

## 准备配置

插件管理员需要把安装包中的 `otel-http.config.toml` 复制到部署配置目录并修改，至少替换占位 endpoint：

```toml
endpoint = "https://collector.example:4318/v1/traces"
allow_insecure = false
attribute_mode = "metadata-only"

[action_kinds]
default = false
"process.exec" = true
"process.exit" = true
"llm.request" = true
"llm.response" = true
```

- 明文 `http://` 必须显式设置 `allow_insecure = true`；生产环境应使用 HTTPS。
- mTLS 的 `tls_client_cert_path` 和 `tls_client_key_path` 必须同时设置，且只适用于 HTTPS。
- `[action_kinds]` 是出境白名单；`default` 必须为 `false`，逐项开启允许离开 daemon 的类型。
- `attribute_mode = "metadata-only"` 只发送结构化元数据；`full` 仅发送 daemon 已生成的属性，
  不会自动授权生成可选正文。
- `[[headers]]` 中的凭据以明文保存在配置文件中，只应通过 HTTPS 发送并限制文件权限。

### LLM 正文授权

发送 LLM request body 需要三层同时允许：

```toml
# operator config
[semantic_retention.l0_llm_call]
request_content = "canonical_blocks"
request_body_export = "canonical_json"
request_body_export_max_bytes = 131072

# otel-http plugin config
attribute_mode = "full"
```

此外 `[action_kinds]` 必须允许 `llm.request`。这三层分别控制本地保留、动作属性生成和
出境，不能互相替代。工具结果正文同理，需要 operator 配置中的
`tool_results_enabled = true`、`tool_result_content_export = "canonical_json"`、大小上限、`attribute_mode = "full"`，
并允许 `llm.tool_result`。插件管理员需要在启用前确认本地留存、Collector 和传输链路都能承载敏感内容。

## 加载

```bash
sudo target/release/actraild --config operator.conf plugin load \
  --manifest /absolute/path/otel-http.plugin.toml \
  --plugin-config /etc/actrail/plugins/otel-http.config.toml \
  --instance my.otel-http

sudo target/release/actraild --config operator.conf plugin status \
  --instance my.otel-http
```

加载成功只说明配置和实例创建成功。endpoint 不可达属于运行期下游故障：实例可以保持
`active`，同时 `last_error`、retry 和 dropped batch 指标报告交付失败。卸载会触发
consumer `finish`，在 `shutdown_flush_deadline_ms` 内尝试发送尾批次，不会重启 daemon。

## 识别 Agent 产品与关联 trace

当某个进程首次产生 `llm.request` 时，AcTrail 会输出一个 `agent.identity` span。
该 span 与其他 span 共用 OTLP `traceId`；消费者可用 `traceId` 和 `actrail.process.id`
将 Agent 身份关联回具体进程。`action_kinds` 白名单需要同时放行 `agent.identity`，
否则该 span 不会出境。

span 上同时给出观测事实与由它推导出的产品结论：

| 属性 | 含义 |
| --- | --- |
| `process.executable` | 同一 `(trace_id, process)` 观测到的二进制，观测事实 |
| `actrail.agent.type` | 由该二进制推导出的 Agent 产品名，**推断结果** |
| `actrail.agent.type.source` | 推断来源，见下表 |

`actrail.agent.type.source` 的取值决定这个产品名有多可信：

| 取值 | 含义 |
| --- | --- |
| `known_product` | 二进制名或其所在的包路径命中了已知 Agent 产品，可直接用于分组 |
| `executable` | 未命中已知产品，退化为程序名。`python3 /srv/app/main.py` 这类由解释器托管的 Agent 会落在这里，取值可能只是 `python3` |

一个进程可能连续 exec 多次：shebang 脚本会先报告脚本本身、再报告解释器最终落到的二进制
（`#!/usr/bin/env node` 会经由 `env` exec 到 `node`）。此时保留信息量更大的那一个 ——
已命中已知产品的二进制不会被未命中的覆盖；而 `sh -c "codex ..."` 这类包装器仍然会被它
启动的程序覆盖。

未观测到该进程的 exec 时（例如挂载到一个已在运行的进程），上述三个属性都不会出现。

`metadata-only` 模式保留这几个身份字段，但仍会删除正文类属性。

## 排错

| 现象 | 原因或处理 |
| --- | --- |
| `plaintext endpoint requires allow_insecure = true` | 显式确认明文，或改用 HTTPS |
| `action_kinds.default must be false` | 出境必须逐项白名单，不支持默认全部发送 |
| TLS options require HTTPS | 删除 TLS 路径或改为 HTTPS endpoint |
| `active` 但 Collector 无数据 | 查看 `last_error`、retry、dropped 与 `observed_records` |
| `observed_records` 为 0 | 当前 trace 的 action kind 没有被允许 |
