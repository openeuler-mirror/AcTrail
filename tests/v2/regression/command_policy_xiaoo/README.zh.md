# 真实 Xiaoo 动态命令策略回归

该 v2 回归使用 Web Configuration 为 `wasm.command-policy-dynamic` 只授予 `/usr/bin/bash` 的 deny scope，并发布 `args=["-c","*"]` 的规则，再驱动真实 Xiaoo 发起 Bash tool call。末尾 `*` 匹配任意剩余参数；测试不伪造 Agent 或 seccomp 通知。

本地确定性流式 provider 只负责向真实 Xiaoo 返回一次 Bash tool request。Xiaoo 工具进程、`execve`、seccomp user notification、daemon 决策、SQLite 审计和 Web 告警均走实际运行路径。

## 自动断言

测试依次验证：

1. Web load 将 executable scope 转换为 `command-policy.rules.apply:kind=deny,path=/usr/bin/bash`。
2. 同时包含授权规则和越权规则的 Configuration Test 以 all-or-nothing 方式拒绝，插件内存配置和 daemon revision 均不变化。
3. Web Update 生成稳定规则 ID `command-dynamic-1`，带 `-c` argv 的 dry-run 返回 owner、decision 和 revision。
4. 同一个 `/usr/bin/bash` 以 `--version` 运行时不命中 `[-c,*]`，保持允许。
5. `actrailctl launch` 请求 `enforcement-command-execution-seccomp` 后，真实 Xiaoo Bash tool 返回 `EPERM`，marker 不存在。
6. CLI 事件包含 `seccomp-user-notify` Enforcement；Web alerts 包含 high severity 的 `command.execution.boundary-violation`。
7. Web 卸载 policy owner 后，第二次真实 Xiaoo Bash tool 成功并创建 marker。

## 运行

从仓库根目录通过聚合入口运行：

~~~bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case command_policy_xiaoo
~~~

也可以独立运行：

~~~bash
sudo -E python3.11 \
  tests/v2/regression/command_policy_xiaoo/run_e2e.py
~~~

公共 runner 会先执行 `scripts/install-release.sh`，因此使用 release 二进制和最新官方 WASM 包。成功时默认清理 case workspace、日志、daemon、Web 和本地 provider。调试失败现场可增加 `--no-cleanup`。

## 手动测试

手动测试不调用自动 runner，也不创建本地 provider、Xiaoo 配置或 AcTrail
operator patch；它直接使用已安装的 release、AcTrail 默认配置和 Xiaoo 现有配置。
以下命令均从仓库根目录执行。

> `actraild init -f` 会覆盖默认 operator config，`actrailctl clean` 会清理默认
> AcTrail 数据。只在专用测试环境执行。若 release 或官方插件尚未安装，先运行
> `scripts/install-release.sh`，不要用旧产物继续测试。

### 步骤 1：检查前提并启动 AcTrail

在终端 A 执行：

~~~bash
sudo -E bash
set -euo pipefail

REPO="$(pwd -P)"
MARKER="$REPO/temp/command-policy-xiaoo-manual.marker"
BASH_EXECUTABLE="$(
  readlink -f "${COMMAND_POLICY_XIAOO_E2E_BASH:-/usr/bin/bash}"
)"
XIAOO_BIN="$(
  readlink -f "${COMMAND_POLICY_XIAOO_E2E_BINARY:-$(command -v xiaoo)}"
)"
LAUNCH_TIMEOUT_SECONDS="${COMMAND_POLICY_XIAOO_E2E_LAUNCH_TIMEOUT_SECONDS:-180}"
PLUGIN_ROOT="${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}"

test "$(id -u)" -eq 0
test -x "$BASH_EXECUTABLE"
test -x "$XIAOO_BIN"
[[ "$LAUNCH_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]
command -v actraild actrailctl actrailviewer actrailweb >/dev/null
test -f "$PLUGIN_ROOT/command-policy-dynamic/command-policy-dynamic.plugin.toml"
test -f "$PLUGIN_ROOT/command-policy-dynamic/command-policy-dynamic.config.json"
test -f "$PLUGIN_ROOT/command-policy-dynamic/config.schema.json"
test -f "$PLUGIN_ROOT/command-policy-dynamic/component-command-policy-dynamic.wasm"
printf 'bash=%s\nxiaoo=%s\n' "$BASH_EXECUTABLE" "$XIAOO_BIN"
mkdir -p "$REPO/temp"
rm -f "$MARKER"

"$XIAOO_BIN" --cli run --no-tools --max-turns 1 \
  --prompt 'Reply with exactly "XIAOO_COMMAND_POLICY_READY" and nothing else.'

actraild init -f
actraild stop
actrailctl clean
actraild start
actraild status
actrailctl doctor
~~~

预期结果：Xiaoo 使用现有配置返回 `XIAOO_COMMAND_POLICY_READY`；四个 AcTrail
程序与 command-policy 官方包可用；daemon 成功启动，doctor 不报告 command
control、seccomp notify 或 storage 错误。任何前提失败都先修复，不能生成临时 Xiaoo
配置或切换到其他 Agent 绕过。

### 步骤 2：启动 Web、加载插件并发布规则

在终端 B 执行：

~~~bash
sudo -E actrailweb
~~~

打开 `http://127.0.0.1:18080`，进入 **Plugins**：

1. 点击 **Refresh**，找到 `wasm.command-policy-dynamic`。
2. 点击 **Configure & load**；Runtime instance name 保持
   `wasm.command-policy-dynamic`。
3. Executable scope 填 `$BASH_EXECUTABLE` 的实际输出，默认是
   `/usr/bin/bash`；Rule types 只保留 **Deny**。
4. 点击 **Load plugin**，确认状态为 `Active`，Host grants 包含
   `command-policy.rules.apply:kind=deny,path=/usr/bin/bash`。
5. 在 **Plugin command** 依次输入 `rule`、`dry-run`、`/usr/bin/bash`、
   `--args-json`、`["-c","probe"]` 并发送。确认返回 `matched=false`，记下
   `source_revision`。
6. 在 **Configuration** 添加以下两条规则并点击 **Test configuration**：

~~~json
{
  "rules": [
    {
      "decision": "deny",
      "executable": "/usr/bin/bash",
      "args": ["-c", "*"],
      "priority": 20
    },
    {
      "decision": "deny",
      "executable": "/srv/not-granted-command",
      "priority": 10
    }
  ]
}
~~~

预期 Test 失败并明确指出缺少 `/srv/not-granted-command` 的 apply grant；当前配置
仍为 `{"rules":[]}`，再次 dry-run 的 `source_revision` 不变。

删除越权规则，只保留 `/usr/bin/bash` 规则，再依次点击 **Test configuration** 和
**Update configuration**。更新后确认规则 ID 为 `command-dynamic-1`，然后用
`["-c","printf test"]` 再次 dry-run，预期包含：

~~~text
matched=true decision=deny rule_id=command-dynamic-1 owner=wasm.command-policy-dynamic
~~~

原子拒绝、有效配置测试、Update 和最终 dry-run 任一项失败都不得继续。

### 步骤 3：验证 argv 范围并运行真实 Xiaoo

回到终端 A：

~~~bash
NONMATCHING_OUTPUT="$(
  timeout "$LAUNCH_TIMEOUT_SECONDS" \
    actrailctl launch --name v2-command-policy-bash-nonmatching-args -- \
      "$BASH_EXECUTABLE" --version 2>&1
)"
printf '%s\n' "$NONMATCHING_OUTPUT"
grep -F 'GNU bash' <<<"$NONMATCHING_OUTPUT"

rm -f "$MARKER"
DENIED_OUTPUT="$(
  timeout "$LAUNCH_TIMEOUT_SECONDS" \
    actrailctl launch --name v2-command-policy-xiaoo-denied -- \
      "$XIAOO_BIN" \
        --cli run \
        --tools bash \
        --max-turns 3 \
        --debug \
        --prompt "Use the Bash tool exactly once to run: printf ACTRAIL_XIAOO_COMMAND_OK > '$MARKER'. Then report its exact operating-system result." \
      2>&1
)"
printf '%s\n' "$DENIED_OUTPUT"

grep -F 'enforcement-command-execution-seccomp' <<<"$DENIED_OUTPUT"
grep -Eiq 'permission denied|operation not permitted' <<<"$DENIED_OUTPUT"
test ! -e "$MARKER"

DENIED_TRACE_ID="$(
  sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p' \
    <<<"$DENIED_OUTPUT"
)"
test "$(printf '%s\n' "$DENIED_TRACE_ID" | sed '/^$/d' | wc -l)" -eq 1
printf 'denied_trace=trace-%s\n' "$DENIED_TRACE_ID"
~~~

预期 Bash `--version` 被允许；真实 Xiaoo 的 Bash tool 返回 EPERM，marker 不存在，
且 launch 输出中只包含一个 denied trace ID。

### 步骤 4：检查治理证据

~~~bash
actrailctl list-traces
actrailviewer events --trace-id "$DENIED_TRACE_ID" |
  rg 'Enforcement|seccomp-user-notify|/usr/bin/bash|denied|command-dynamic-1'
~~~

预期同一 trace 至少有一条 Enforcement 事件，包含 `seccomp-user-notify`、
`/usr/bin/bash`、`denied` 和 `command-dynamic-1`。

在 Web 的 **Stats → Alerts** 刷新并打开该 trace 的告警。预期存在 high severity 的
`command.execution.boundary-violation`，producer 是 `actraild.enforcement`，payload
中的 executable 和 rule ID 分别为 `/usr/bin/bash` 与 `command-dynamic-1`。告警
尚未出现时可在 15 秒内刷新；超过时限即为失败。

### 步骤 5：卸载 owner 并验证恢复

在 Web 的 **Plugins** 中卸载 `wasm.command-policy-dynamic`，确认实例不再是
`Active`。回到终端 A：

~~~bash
rm -f "$MARKER"
ALLOWED_OUTPUT="$(
  timeout "$LAUNCH_TIMEOUT_SECONDS" \
    actrailctl launch --name v2-command-policy-xiaoo-owner-unloaded -- \
      "$XIAOO_BIN" \
        --cli run \
        --tools bash \
        --max-turns 3 \
        --debug \
        --prompt "Use the Bash tool exactly once to run: printf ACTRAIL_XIAOO_COMMAND_OK > '$MARKER'. Then report the file content." \
      2>&1
)"
printf '%s\n' "$ALLOWED_OUTPUT"

if grep -Eiq 'permission denied|operation not permitted' <<<"$ALLOWED_OUTPUT"; then
  printf 'Bash remained denied after owner unload\n' >&2
  exit 1
fi
test "$(sed -n '1p' "$MARKER")" = ACTRAIL_XIAOO_COMMAND_OK
~~~

预期 Xiaoo 的 Bash tool 成功，marker 内容严格等于
`ACTRAIL_XIAOO_COMMAND_OK`，证明 owner 卸载后动态规则已撤销。

### 步骤 6：清理

先在终端 B 按 `Ctrl-C` 停止 Web，再在终端 A 执行：

~~~bash
actraild stop
actrailctl clean
rm -f "$MARKER"
exit
~~~

预期 daemon 停止，测试 trace 和 marker 被删除。

## 可配置入口

- `COMMAND_POLICY_XIAOO_E2E_BINARY`：真实 Xiaoo 绝对路径；默认从 `PATH` 查找。
- `COMMAND_POLICY_XIAOO_E2E_BASH`：治理目标；默认 `/usr/bin/bash`。
- `COMMAND_POLICY_XIAOO_E2E_WEB_HOST`：Web 监听地址；自动测试默认 `127.0.0.1`。
- `COMMAND_POLICY_XIAOO_E2E_WEB_PORT`：Web 监听端口；自动测试默认 `0`，由系统动态分配。
- `COMMAND_POLICY_XIAOO_E2E_READY_TIMEOUT_SECONDS`：服务启动时限；默认 15 秒。
- `COMMAND_POLICY_XIAOO_E2E_EVIDENCE_TIMEOUT_SECONDS`：Enforcement/alert 等待时限；默认 15 秒。
- `COMMAND_POLICY_XIAOO_E2E_LAUNCH_TIMEOUT_SECONDS`：单次 Xiaoo launch 时限；默认 180 秒。

所有时限必须为正数。缺少真实 Xiaoo、Bash、release 产物或治理证据时测试
fail-fast，不返回虚假通过或降级结果。
