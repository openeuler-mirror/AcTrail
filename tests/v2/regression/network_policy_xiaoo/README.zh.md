# 真实 Xiaoo 精确网络策略回归

该回归启动本地确定性 provider，取得其真实动态
`127.0.0.1:<port>`，再通过 Web 为 `wasm.network-policy-dynamic` 授予同一个
精确 endpoint 的 deny scope，并控制真实 Xiaoo 到 provider 的 `connect(2)`。
`127.0.0.1:*` 是 IP 全端口 selector，不属于本测例。

## 自动覆盖

测例按同一个 provider、Xiaoo 配置和 daemon 依次验证：

1. 刷新后的默认 operator config 经隔离 patch 启用
   `enforcement-network-connect-seccomp`；provider 启动并报告动态端口后，最新安装
   的官方 WASM 包才通过 Web 以该精确 endpoint 的 deny grant 加载。
2. 空规则配置的 dry-run 返回 `matched=false decision=allow`，真实 Xiaoo 完成
   provider、Bash tool 和最终响应闭环，marker 内容正确。
3. Web Configuration Test/Update 发布 `xiaoo-provider-deny`，规则的 `remote`
   严格等于 provider endpoint；dry-run 返回稳定 owner、rule revision 和递增的
   source revision。
4. 第二次真实 Xiaoo 以非零状态退出并报告 provider connection failure，marker
   不存在；launch 明确选择 network-connect enforcement 和 seccomp user-notify。
5. SQLite 中该 trace 的每一条 provider endpoint `network-control` 事件都必须是
   `inet`/`network-action`/`connect`、`result=-EPERM`，且
   `payload.remote` 和已解析规则的 `policy_remote_scope` 都严格等于真实
   `127.0.0.1:<port>`。
6. Web 把配置清回空规则后，dry-run 回到 allow 且 source revision 再次递增；
   第三次真实 Xiaoo 恢复成功并重新创建 marker。

grant 越权、selector 重叠、gray/cache/timeout/overload、owner 或 decider 卸载由
`tests/plugins/network-policy-dynamic/run_e2e.py` 覆盖。本测例证明精确 endpoint
规则控制真实 Xiaoo connect，但不把同 IP 的第二端口排他性、域名、CIDR、DNS、
TLS SNI、代理最终目标、`sendto(2)`、AF_UNIX 或继承连接误写成当前覆盖。

## 自动运行

独立运行：

~~~bash
sudo -E python3 tests/v2/regression/network_policy_xiaoo/run_e2e.py
~~~

聚合运行：

~~~bash
sudo -E python3 tests/v2/regression/test_all.py --case network_policy_xiaoo
~~~

公共 runner 会先执行 `scripts/install-release.sh`，测例从
`${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}/network-policy-dynamic` 复制本次
安装的官方包，不使用仓库内可能陈旧的预构建 WASM。缺少真实 Xiaoo、release
产物、官方插件或治理证据时均 fail-fast。

## 手动测试

手动测试使用三个终端和 Web UI，以下操作由人工分步执行，不调用任何
`run_e2e.py`。测试直接刷新 AcTrail 默认配置和数据，只应在专用测试环境
执行。若 release 或官方插件尚未安装，先从仓库根目录运行
`scripts/install-release.sh`。

> `actraild init -f` 会覆盖默认 operator config，`actrailctl clean` 会清理
> 默认 AcTrail 数据；共享或生产环境不得执行以下流程。

### 步骤 1：检查前提并启动 AcTrail

在终端 A 执行：

~~~bash
sudo -E bash
set -euo pipefail

REPO="$(pwd -P)"
MARKER="$REPO/temp/network-policy-xiaoo-manual.marker"
XIAOO_CONFIG="$REPO/temp/network-policy-xiaoo-manual.toml"
XIAOO_BIN="$(readlink -f "${NETWORK_POLICY_XIAOO_E2E_BINARY:-$(command -v xiaoo)}")"
LAUNCH_TIMEOUT_SECONDS="${NETWORK_POLICY_XIAOO_E2E_LAUNCH_TIMEOUT_SECONDS:-180}"
PLUGIN_ROOT="${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}"

test "$(id -u)" -eq 0
test -x "$XIAOO_BIN"
[[ "$LAUNCH_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]
grep -qw user_notif /proc/sys/kernel/seccomp/actions_avail
command -v actraild actrailctl actrailviewer actrailweb jq >/dev/null
test -f "$PLUGIN_ROOT/network-policy-dynamic/network-policy-dynamic.plugin.toml"
test -f "$PLUGIN_ROOT/network-policy-dynamic/network-policy-dynamic.config.json"
test -f "$PLUGIN_ROOT/network-policy-dynamic/config.schema.json"
test -f "$PLUGIN_ROOT/network-policy-dynamic/component-network-policy-dynamic.wasm"
mkdir -p "$REPO/temp"
rm -f "$MARKER" "$XIAOO_CONFIG"

actraild init -f
actraild stop
actrailctl clean
actraild start
actraild status
actrailctl doctor
~~~

预期 daemon 成功启动，doctor 不报告 network control、seccomp notify、storage 或
插件目录错误。任一前提失败都先修复，不切换模拟 Agent 或旧 WASM。

### 步骤 2：启动 provider 并记录真实 endpoint

在终端 B 从仓库根目录执行并保持前台运行：

~~~bash
sudo -E bash
REPO="$(pwd -P)"
MARKER="$REPO/temp/network-policy-xiaoo-manual.marker"
python3 tests/support/llm-http-proxy/provider_proxy.py \
  --mode local-stream --bind-host 127.0.0.1 --bind-port 0 \
  --local-stream-response-text ACTRAIL_XIAOO_NETWORK_POLICY_OK \
  --local-stream-reasoning-tokens 1 \
  --local-tool-command "printf ACTRAIL_XIAOO_NETWORK_OK > $MARKER"
~~~

复制终端打印的 `proxy_base_url=http://127.0.0.1:<port>`。回到终端 A，把下面的
示例替换为实际值：

~~~bash
PROVIDER_URL='http://127.0.0.1:实际端口'
PROVIDER_ENDPOINT="${PROVIDER_URL#http://}"
[[ "$PROVIDER_ENDPOINT" =~ ^127[.]0[.]0[.]1:[0-9]+$ ]]

tee "$XIAOO_CONFIG" >/dev/null <<EOF
[llm]
provider = "deepseek"
model = "deepseek-chat"
api_key_env = "ACTRAIL_XIAOO_NETWORK_POLICY_KEY"
api_base = "$PROVIDER_URL"
max_tokens = 128
context_window = 32768
reasoning_effort = "off"
EOF

unset ALL_PROXY HTTPS_PROXY HTTP_PROXY all_proxy https_proxy http_proxy
export NO_PROXY=127.0.0.1,localhost
export no_proxy=127.0.0.1,localhost
export ACTRAIL_XIAOO_NETWORK_POLICY_KEY=local-test-key
printf 'exact_endpoint=%s\n' "$PROVIDER_ENDPOINT"
~~~

### 步骤 3：通过 Web 加载空策略

在终端 C 执行 `sudo -E actrailweb`，打开
`http://127.0.0.1:18080` 并进入 **Plugins**：

1. Refresh 后找到 `wasm.network-policy-dynamic`，点击
   **Configure & load**。
2. Runtime instance name 保持 `wasm.network-policy-dynamic`，Rule types 只
   保留 **Deny**；**Remote endpoint scope** 填终端 A 输出的精确
   `127.0.0.1:<port>`，禁止填写 `127.0.0.1:*`。
3. Load 后确认状态为 `Active`，Host grants 包含
   `network-policy.rules.apply:kind=deny,remote=127.0.0.1:<port>`。
4. Configuration 应为 `{"rules":[]}`。在 **Plugin command** 依次输入
   `rule`、`dry-run`、精确 endpoint 并发送，确认返回
   `matched=false decision=allow rule_id=none owner=none`，记下
   `source_revision`。

### 步骤 4：运行真实 Xiaoo allow 基线

在终端 A 执行一次真实 launch：

~~~bash
rm -f "$MARKER"
timeout "$LAUNCH_TIMEOUT_SECONDS" \
  actrailctl launch \
    --host-ebpf disabled \
    --seccomp-notify required \
    --name v2-network-policy-xiaoo-manual-baseline -- \
    "$XIAOO_BIN" --cli run \
      --config "$XIAOO_CONFIG" \
      --tools bash \
      --max-turns 3 \
      --debug \
      --prompt 'Use the Bash tool exactly once, then report its operating-system result.'
test "$(sed -n '1p' "$MARKER")" = ACTRAIL_XIAOO_NETWORK_OK
~~~

人工确认输出包含 `enforcement-network-connect-seccomp`、
`seccomp_notify:enabled` 和 `ACTRAIL_XIAOO_NETWORK_POLICY_OK`，并记住
`trace-<N>`。终端 B 应看到 tool call 和 final response 两次 provider 请求。

### 步骤 5：发布精确 deny 并重跑真实 Xiaoo

在 Web 的 **Configuration** 中填入下面配置，把端口替换为
`$PROVIDER_ENDPOINT` 的实际端口：

~~~json
{
  "rules": [
    {
      "rule_id": "xiaoo-provider-deny",
      "decision": "deny",
      "remote": "127.0.0.1:实际端口"
    }
  ]
}
~~~

依次点击 **Test configuration** 和 **Update configuration**。再次 dry-run 同一个
精确 endpoint，确认 `matched=true decision=deny`，owner 和 rule ID 正确，
`rule_revision` 非空且 `source_revision` 已前进。

回到终端 A，先执行：

~~~bash
rm -f "$MARKER"
set +e
~~~

然后用终端历史重新执行步骤 4 的 `timeout ... actrailctl launch` 命令，只把
name 改成 `v2-network-policy-xiaoo-manual-denied`。命令结束后立即记录并恢复
严格模式：

~~~bash
DENIED_RC=$?
set -e
test "$DENIED_RC" -ne 0
test "$DENIED_RC" -ne 124
test ! -e "$MARKER"
~~~

人工确认输出仍选择 network-connect seccomp，但报告 `connection failed` 和
`LLM provider error`；终端 B 不应出现新请求。记下本轮唯一的 denied trace ID。

### 步骤 6：检查治理证据

在终端 A 把数字替换为 denied trace ID；若事件尚未落盘，可在 15 秒内手动重试：

~~~bash
DENIED_TRACE_ID=实际数字
actrailctl list-traces
actrailviewer --output-format json events --trace-id "$DENIED_TRACE_ID" |
  jq --arg endpoint "$PROVIDER_ENDPOINT" '
    [.events[]
      | select(
          .collector == "network-control" and
          .variant == "net" and
          .payload.remote == $endpoint)
      | {
          remote: .payload.remote,
          transport: .payload.transport,
          result: .payload.result,
          subject: .payload.metadata.subject,
          operation: .payload.metadata.operation,
          decision: .payload.metadata.decision,
          source: .payload.metadata.decision_source,
          rule_id: .payload.metadata.rule_id,
          owner: .payload.metadata.policy_owner_instance_id,
          scope: .payload.metadata.policy_remote_scope,
          revision: .payload.metadata.rule_revision
        }]
  '
~~~

结果必须非空；每一项都应是 `transport="inet"`、`result=-1`、
`subject="network-action"`、`operation="connect"`、`decision="deny"`、
`source="rule"`，rule ID 和 owner 正确，且 `remote`、`scope` 都严格等于
实际 `127.0.0.1:<port>`。revision 必须与 deny dry-run 一致。

### 步骤 7：清空规则并验证恢复

在 Web 中把 Configuration 改回 `{"rules":[]}`，依次 Test 和 Update；再次
dry-run 精确 endpoint，确认返回 default allow 且 source revision 前进。

回到终端 A，删除 marker，并从历史重跑步骤 4 的 launch，只把 name 改为
`v2-network-policy-xiaoo-manual-restored`。预期再次成功、marker 内容正确，终端
B 恢复收到两次请求。最后在 Web 卸载插件并确认实例不再是 `Active`。

### 步骤 8：清理

先在终端 B、C 按 `Ctrl-C` 停止 provider 和 Web，再在终端 A 执行：

~~~bash
actraild stop
actrailctl clean
rm -f "$MARKER" "$XIAOO_CONFIG"
exit
~~~

预期 daemon 停止，测试 trace、marker 和临时 Xiaoo 配置被删除。

## 可配置入口

- `NETWORK_POLICY_XIAOO_E2E_BINARY`：真实 Xiaoo 绝对路径，默认从 `PATH` 查找。
- `NETWORK_POLICY_XIAOO_E2E_WEB_HOST`：自动测试 Web 监听地址，默认
  `127.0.0.1`。
- `NETWORK_POLICY_XIAOO_E2E_WEB_PORT`：自动测试 Web 端口，默认 `0`，由内核
  选择可连接的 loopback 端口。
- `NETWORK_POLICY_XIAOO_E2E_READY_TIMEOUT_SECONDS`：provider/Web 启动时限，
  默认 15 秒。
- `NETWORK_POLICY_XIAOO_E2E_EVIDENCE_TIMEOUT_SECONDS`：SQLite 治理证据等待
  时限，默认 15 秒。
- `NETWORK_POLICY_XIAOO_E2E_LAUNCH_TIMEOUT_SECONDS`：单次 Xiaoo launch 时限，
  默认 180 秒。
- `ACTRAIL_PLUGIN_DIR`：自动 runner 安装并由测例复制的官方插件根目录，必须是
  绝对路径；默认 `$HOME/.actrail/plugins`。

所有时限必须为正数。
