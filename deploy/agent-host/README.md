# Agent Host 接入

`crates/adapters/agent_host` 管理 agent 专用识别、接入配置、插件检查和启动环境。接入负责报告原生生命周期事实；idle_detector 消费这些事实，其检测和告警配置单独管理。

## OpenCode 配置

将 `crates/adapters/agent_host/assets/opencode/` 下的 `plugins/` 和 `lib/` 一起安装到接入目录，例如 `/usr/local/lib/actrail/opencode/`。

创建独立文件 `agent-host.toml`：

```toml
[opencode]
enabled = true
plugin_dir = "/usr/local/lib/actrail/opencode"
```

`plugin_dir` 的相对路径相对于该配置文件目录。启用 OpenCode 接入时必须指定目录；启动 OpenCode 前验证全部插件文件。

```bash
actrailctl launch --agent-host-config ./agent-host.toml --name opencode -- opencode
```

省略 `--agent-host-config` 时不注入接入插件。显式配置的读取、解析或启用插件检查失败会使 launch 报错。daemon 的 operator 配置不包含 OpenCode 专用字段，也不读取该文件。

接入模块为匹配的 OpenCode 命令注入 `OPENCODE_CONFIG_DIR`、`ACTRAIL_AGENT_LIFECYCLE_ENABLED=true`、`ACTRAIL_TRACE_ID` 和 `ACTRAIL_CONTROL_SOCKET`。OpenCode 配置目录会与项目及全局配置叠加。`opencode run` 和 `opencode serve` 也可通过同一 launch 入口启动。

## 生命周期事实

| OpenCode 事件 | 上报事实 |
| --- | --- |
| `message.updated` 中原生用户消息 | turn started，task_id 使用原生 message ID |
| `session.idle` 或 idle 的 `session.status` | turn completed |
| permission/question asked | interaction requested |
| permission/question replied 或 rejected | interaction resolved |
| `session.deleted` | session closed |
| `chat.headers`（网络请求前） | model started |
| `message.part.updated` 的 `step-finish` | model completed |
| 工具 part 首次 `running` | tool started |
| 工具 part 的 `completed` 或 `error` | tool completed |

每条事实包含 trace_id、原生 session_id 和适用的 task_id、interaction_id。不同 session 的轮次独立；不从 task 字符串解析 session。时间戳表示 adapter 观测转换的时刻，受 JavaScript 时钟精度限制；UDS 排队不会重新赋予时间戳。投递结果不决定已观察到的原生任务状态。

缺少原生用户消息时不会虚构任务身份。模型与工具的在途计数用于排除正常等待。模型重试保留同一次在途状态，后台标题请求不计入用户任务；工具按原生 call ID 去重，异常由 error 终态结束等待，不依赖 tool.execute.after。权限等待的处理以原生请求及解决事件为依据，adapter 不批准任何操作。

支持 `permission.*` / `question.*` 与对应的 `.v2.*` 事件。部分 OpenCode 1.18.x 权限事件不转发给 server plugin，接入保留对本地权威 permission 列表的对账：默认 250 ms，可用 `ACTRAIL_OPENCODE_PERMISSION_POLL_INTERVAL_MS` 设置不低于 100 ms 的间隔。

## 协议与验证

daemon 与 adapter 一起部署，使用含显式 session_id 的 `report_turn_lifecycle_v2`、`report_user_interaction_v2`、`report_work_lifecycle` 和 `report_session_closed` 协议。

外部 agent host 可记录 session 关闭，时间取命令执行时刻：

```bash
actrailctl session-closed --trace-id trace-1 --session-id ses_native_id
```

真实验收入口为 `tests/process/idle-detection/e2e.py`，使用安装的 OpenCode、刷新默认配置、独立接入配置和本地 MaaS。应核对原生消息/session 身份、真实权限等待、多 session 隔离、工具失败及停滞恢复；缺失上报不能用“没有区间”作为通过证据。
