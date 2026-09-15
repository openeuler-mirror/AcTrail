<!-- Documents deployment of the OpenCode agent-host adapter. -->
# OpenCode Agent Host 适配器

此部署目录仅包含 OpenCode 集成。插件使用 OpenCode 的结构化事件 API，直接向 AcTrail 控制 socket 报告生命周期变化；不会安装或调用通用 shell hook。

OpenCode 提供结构化的本地插件事件 API。`deploy/agent-host/opencode/plugins/` 中的适配器同时消费当前的 `permission.v2.*` / `question.v2.*` 事件及其不带版本号的旧版等价事件；它不会解析 TUI 中的 `allow` 文本。
OpenCode 1.18.x 可能能够渲染 V2 权限提示，但不会将 `permission.v2.*` 事件转发给 server plugin。因此，适配器还会每 250 ms（可通过 `ACTRAIL_OPENCODE_PERMISSION_POLL_INTERVAL_MS` 配置，最小值为 100 ms）对 OpenCode 本地权威的 `GET /permission` 列表进行对账。插件使用 OpenCode 注入的进程内 client transport；AcTrail 自身的报告仍然通过 Unix socket 直接进行，不会启动 shell 或子进程。
权限/提问请求 ID 与 session ID 会组合成一个稳定的 AcTrail interaction ID，因此重复事件以及来自其他 session 的回复都会被忽略。被拒绝的权限/提问只会解决对应的 interaction，不会关闭外层的 OpenCode task。

插件还会为每个 OpenCode turn 报告权威生命周期。`properties.info.role=user` 的 `message.updated` 会启动一个新的 `<task-prefix>:session:<session-id>:turn:<number>` task。只有在没有待处理 interaction 时，`session.idle`（或状态为 `idle` 的结构化 `session.status`）才会完成该 turn。在审批或提问期间观察到的 idle 事件会被延后处理。权限回复只会解决对应 interaction；之后仍然需要一个允许操作完成后的 idle 事件来完成 turn。因此，回到 OpenCode 输入界面会关闭当前 turn，同时保持 CLI 可继续接收下一条用户消息；下一条消息会启动新的 task ID。

请将 `plugins/` 和 `lib/` 目录一起安装到默认目录 `/usr/local/lib/actrail/opencode/` 下；如果部署在其他位置，则覆盖 `opencode_plugin_dir`。启用自动注入：

```toml
[idle_detection]
enabled = true
opencode_auto_inject = true
threshold_secs = "30s"
```

默认调用会启动交互式 TUI：

```bash
sudo -E actrailctl launch --name opencode-prod -- \
  opencode
```

如需执行无头脚本化 turn，可直接传入 OpenCode 标准的 `run` 子命令：

```bash
sudo -E actrailctl launch --name opencode-prod -- \
  opencode \
  run "完成当前任务"
```

当 `idle_detection.enabled = true` 且 `opencode_auto_inject = true` 时，`actrailctl launch -- opencode` 会检查部署文件，并将 `OPENCODE_CONFIG_DIR` 设置为适配器目录。OpenCode 会自动加载该目录中约定布局的 `plugins/actrail-idle-plugin.js`；支持模块位于 `lib/`，不在 OpenCode 的自动插件扫描范围内。
该自定义目录会与用户正常的全局/项目配置叠加，不会替换这些配置。`ACTRAIL_TRACE_ID` 和 `ACTRAIL_CONTROL_SOCKET` 仍由 `actrailctl launch` 提供。配置的目录必须包含 `plugins/actrail-idle-plugin.js` 以及匹配的 `lib/` 模块。插件直接通过 Unix domain socket 报告，不会启动 shell 或 `actrailctl` 子进程。

事件结构固定遵循当前 OpenCode 合约：asked 事件包含 `sessionID` 和请求 `id`；权限列表对账使用相同字段；当前 V2 权限/提问回复包含 `sessionID` 和 `requestID`，而旧版权限回复可能使用 `permissionID`。
权限回复包括 `once`、`always` 或 `reject`；提问回复包括 `question.replied` 或 `question.rejected`。部署基线使用 OpenCode 1.15.13 或更高版本；生产环境请固定精确版本，并在升级后运行 fixture 测试（`node tests/process/idle-detection/opencode-adapter.test.mjs`）。参考文档为 [OpenCode 插件事件 API](https://dev.opencode.ai/docs/plugins/) 和 [权限 API](https://dev.opencode.ai/docs/permissions/)；仓库中的 fixture 不声称可以替代真实 CLI smoke test。
如果插件或任何支持性的 `lib/` 模块缺失，`actrailctl launch` 会在启动 OpenCode 前失败，而不是静默运行并失去可靠的用户等待检测。将 `idle_detection.enabled` 设置为 `false`，即可在不启用此集成的情况下启动 OpenCode。请将完整的 `opencode/` 目录作为一个整体安装。
插件集成必须在 `actrailctl launch` 下运行。launcher 负责 `launch:<trace>` 的 Started/terminal 生命周期，并在前台子进程退出时清理待处理 interaction；插件不会重复报告 terminal 事件。正常 EOF 或正常退出 OpenCode 对应 `turn-completed`；SIGINT 和 SIGTERM 状态码 130、143 对应 `turn-cancelled`；其他非零状态码对应 `turn-failed`。

适配器不会将普通模型延迟、工具输出、`session.status=active` 或静默报告为用户等待。它也不会自动批准任何操作：OpenCode 仍负责处理用户的 `allow`、拒绝或提问回答。只有匹配的结构化权限/提问回复或拒绝事件才会解决 interaction。Session idle/status/error 事件永远不会解决待处理 interaction，因为这些事件可能在权限 promise 仍处于 pending 时发生；显式删除 session 可以解决其待处理 interaction。进程退出清理由 launcher 的 terminal 生命周期负责。

`process.exit` 和 `agent.exit` 仅用于导出，因此可能不会列在持久化的 SQLite action tree 中。当 eBPF 未捕获到退出事件时，daemon 的 live drain 会根据 `/proc` 中的 PID 和启动时间身份，对每个活动成员进行对账；如果发现身份已经消失，会在 idle projection 运行前发出等价的 `process-reconcile` 退出事件。启用调试日志（`RUST_LOG=actrail=debug`）后，请查找 `projecting procfs process exit into live semantic actions` 以及对应的 trace/process ID。Starting/Active 成员扫描最多每秒执行一次；Draining/terminal trace 仍会立即处理。可观察的正确性检查是：仍在运行的子进程应使 `idle_intervals` 保持为空；只有子进程退出并经过配置的阈值后，同一个查询才会打开 interval。

### Ubuntu VM 冒烟测试

下面是真实 TUI smoke test。它假设 `actraild` 和 `actrailweb` 正在运行，并且 operator 配置中的 idle threshold 已知。请从临时项目运行，以隔离权限规则：

```bash
set -euo pipefail
ADAPTER=/usr/local/lib/actrail/opencode
CFG=/etc/actrail/operator.conf
WEB_ORIGIN="${ACTRAIL_WEB_ORIGIN:-http://127.0.0.1:18092}"
WORK=/tmp/actrail-opencode-approval-smoke
mkdir -p "$WORK"
cd "$WORK"
printf '%s\n' '{"permission":{"bash":"ask"}}' > opencode.json
opencode --version
test -f "$ADAPTER/plugins/actrail-idle-plugin.js"
```

在具有真实 TTY 的终端中，直接以前台方式启动交互式集成（不要使用 `script`、通过管道传入 stdin，或将 TUI 放到后台）：

```bash
sudo -E actrailctl --config "$CFG" launch --name opencode-approval-smoke -- \
  opencode
```

让 OpenCode 执行一个未被现有规则允许的 shell 命令，例如输入 `请执行 pwd`。它应停在结构化权限提示处。在第二个终端中，找到带有该 launch name 的最新活动 trace，并在不回答提示的情况下，让查询持续时间超过 `threshold_secs`：

```bash
TRACE_ID="$(curl -fsS "$WEB_ORIGIN/api/traces" | jq -r \
  --arg name opencode-approval-smoke \
  '.traces | map(select(.name == $name and .state == "Active")) | max_by(.id).id')"
test -n "$TRACE_ID"
curl -fsS "$WEB_ORIGIN/api/traces/$TRACE_ID/action-tree" | jq '.idle_intervals'
```

权限提示处于 pending 期间，要求等待时间超过配置阈值后仍没有新的 idle interval。输入 `allow`（或选择 OpenCode 的批准选项）后，命令执行期间仍不应出现由用户等待产生的 interval；OpenCode 返回输入界面后，该已完成 turn 也不应产生 interval。再提交一条用户消息，验证产生新的 `turn-started`，并重复上述检查。该权限请求必须恰好有一对 requested/resolved 事件。
如需单独测试取消，在 TUI 中按 Ctrl-C，并确认 `actrailctl launch` 记录了取消生命周期。正常的 Ctrl-D/EOF 或正常完成则应使用 `turn-completed`。
