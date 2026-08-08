# 动态命令执行策略插件

这个官方 WIT Component 在实例内存中维护命令路由，并通过 command-policy Hostcall 把规则发布给 actraild。它只负责发布 `allow`、`deny`、`gray` 路由；它不是 gray 决策器。如果把该实例配置成 `gray_target`，插件会返回明确错误，daemon 按 fail-closed 语义处理。

```text
Web Configuration / Plugin command
  -> wasm.command-policy-dynamic 内存配置
  -> command-policy AON apply
  -> actraild 合并命令路由
  -> 活动 launch trace 的 execve/execveat seccomp notify
```

动态配置不持久化到 daemon。插件实例或 daemon 重启后，应由运维系统重新提交并确认配置。

## 前置条件

operator 配置必须启用：

```toml
[seccomp_notify]
enabled = true

[command_control]
enabled = true
rules_path = "/etc/actrail/command-control.rules"
default_decision = "allow"
failure_decision = "deny"
audit_enabled = true
audit_default_allow = false
path_max_bytes = 4096
argv_max_count = 128
argv_max_arg_bytes = 8192
argv_max_total_bytes = 65536
pending_decision_max = 64
reusable_cache_max_entries = 4096

[command_control.gray]
timeout_ms = 5000
concurrency_limit = 8
fallback = "deny"
```

launch capture profile 必须同时请求：

```text
proc-lifecycle
enforcement-command-execution-seccomp
```

命令治理只支持 `actrailctl launch`。`track-add` 无法为已运行进程补装 seccomp listener，因此请求该 capability 会 fail-fast。

release 安装程序把候选包放到：

```text
~/.actrail/plugins/command-policy-dynamic/
├── command-policy-dynamic.plugin.toml
├── command-policy-dynamic.config.json
├── config.schema.json
└── component-command-policy-dynamic.wasm
```

安装只让 Web 可以发现包，不会自动加载。

## 通过 Web 加载

1. 打开 **Plugins** 并刷新候选包。
2. 选择 `wasm.command-policy-dynamic`，点击 **Configure & load**。
3. 填写实例 ID，例如 `wasm.command-policy-dynamic`。
4. 在 **Executables this plugin can manage** 添加精确绝对路径或以 `/**` 结尾的范围。
5. 为每个范围选择 Allow、Deny、Ask plugin 中允许发布的决策类型。
6. 加载实例。

Web 后端把这些选择转换成：

```text
command-policy.rules.apply:kind=deny,path=/usr/bin/bash
command-policy.rules.apply:kind=gray,path=/opt/agent-tools/**
```

grant 的范围不会改变规则语义。每条实际规则仍精确匹配一个 tracee namespace executable 路径，并可进一步限制 `argv[1..]`。

## Configuration 格式

```json
{
  "rules": [
    {
      "rule_id": "command-dynamic-1",
      "decision": "deny",
      "executable": "/usr/bin/bash",
      "args": ["-c", "*"],
      "priority": 20
    },
    {
      "decision": "gray",
      "executable": "/usr/bin/python3",
      "priority": 10,
      "gray_target": "remote-command-decider"
    }
  ]
}
```

| 字段 | 要求 | 说明 |
| --- | --- | --- |
| `rule_id` | 新规则可省略 | 省略时按当前实例配置生成稳定的 `command-dynamic-N`。 |
| `decision` | 必填 | `allow`、`deny` 或 `gray`。 |
| `executable` | 必填 | tracee namespace 内的精确绝对路径。 |
| `args` | 可省略 | 匹配 `argv[1..]`。省略表示任意参数，空数组只匹配无额外参数；仅末尾 `"*"` 表示任意剩余参数（包括零个）。 |
| `priority` | 必填 | `i32`；数值越大优先级越高。 |
| `gray_target` | gray 必填 | 当前 active、非自身的 control-decider 实例 ID 字符串。 |

allow/deny 禁止填写 `gray_target`。`"*"` 出现在非末尾位置会拒绝整份配置，其他字符串（包括含 `*` 的字符串）都按字面精确匹配。同一 owner 下 rule ID 和 `(executable, args 逻辑范围)` 都必须唯一；省略 args 与 `["*"]` 是同一逻辑范围。不同 owner 可以发布同一路径；daemon 在 args 匹配的候选中按 priority、更新 sequence 选择有效规则。

先点 **Test configuration**。插件调用 daemon validate；base revision 过期、任一规则非法、grant 越权或 gray target 不可用时整批拒绝。只有 **Update configuration** 的 AON apply 成功后，插件才替换自己的内存配置。

## Plugin command

列出实例配置：

```text
rule
list
```

新增本地 deny：

```text
rule
upsert
deny
/usr/bin/bash
--args-json
["-c","*"]
--priority
20
```

新增 gray 路由：

```text
rule
upsert
gray
/usr/bin/python3
--priority
10
--gray-target
remote-command-decider
```

查询 daemon 当前合并命中结果：

```text
rule
dry-run
/usr/bin/bash
--args-json
["-c","printf test"]
```

删除规则：

```text
rule
delete
command-dynamic-1
```

Configuration 和 Plugin command 操作同一份插件内存。命令修改成功后，Web 重新读取 Configuration，可以看到相同规则和稳定 ID。

## CLI 加载

```bash
export INSTANCE=wasm.command-policy-dynamic
export BASH_EXECUTABLE=/usr/bin/bash

sudo target/release/actraild --config /etc/actrail/actraild.conf plugin load \
  --manifest examples/plugins/wit-component/command-policy-dynamic/plugin.toml \
  --plugin-config examples/plugins/wit-component/command-policy-dynamic/command-policy-dynamic.config.json \
  --grant command-policy.rules.read \
  --grant command-policy.rules.match-dry-run \
  --grant command-policy.rules.validate \
  --grant "command-policy.rules.apply:kind=deny,path=$BASH_EXECUTABLE" \
  --instance "$INSTANCE"
```

```bash
sudo target/release/actraild --config /etc/actrail/actraild.conf plugin cmd \
  --instance "$INSTANCE" -- rule dry-run "$BASH_EXECUTABLE" \
  --args-json '["-c","printf test"]'
```

dry-run 输出应包含 `matched=true`、owner instance、decision、rule revision 和 source revision。

## 真实 Agent 验收

先在 Web 中只授予 `/usr/bin/bash` 的 Deny 范围，然后 Test 并 Update 一条 executable 为 `/usr/bin/bash`、args 为 `["-c","*"]` 的 deny。该规则拒绝 Bash command mode，但不会阻止 `/usr/bin/bash --version`。通过 launch 启动真实 Xiaoo：

```bash
sudo target/release/actrailctl --config /etc/actrail/actraild.conf launch \
  --name command-boundary-xiaoo -- \
  xiaoo --cli run -p \
  '请尝试使用 /usr/bin/bash -c 创建 /tmp/actrail-command-boundary-marker，并如实报告结果。'
```

launch 输出必须包含 `enforcement-command-execution-seccomp`。预期 Xiaoo 报告 `Permission denied` 或 `Operation not permitted`，且 marker 不存在。

查看 trace 证据：

```bash
sudo target/release/actrailviewer events \
  --config /etc/actrail/actraild.conf --trace-id 1
```

应同时存在：

```text
Enforcement execve decision=deny path=/usr/bin/bash result=denied backend=seccomp-user-notify
Alert command.execution.boundary-violation severity=high producer=actraild.enforcement
```

只有显式本地 deny、gray plugin deny 和 gray cache deny 生成 boundary alert。default/failure/fallback deny 仍写 Enforcement，但不会伪装成用户策略越界。

卸载 publisher 后再次让同一活动 trace 执行 Bash：

```bash
sudo target/release/actraild --config /etc/actrail/actraild.conf plugin unload \
  --instance wasm.command-policy-dynamic
```

该 owner 的规则和相关缓存会立即移除；如果没有其他 owner 或静态规则命中，执行恢复 `default_decision`。

上述步骤是验收规范。发布完成声明必须附带本机真实 Xiaoo 输出、marker、Enforcement 和 alert 记录；没有这些证据时只能记录为待验收。

## 路径与 gray 语义

- 绝对 `execve` 做词法规范化；相对路径基于 `/proc/<pid>/cwd`。
- `execveat` 的 `AT_FDCWD` 使用 cwd，其他 dirfd 使用 `/proc/<pid>/fd/<dirfd>`。
- `AT_EMPTY_PATH` 使用 fd link；已删除或无法映射的目标拒绝。
- 不把 symlink 解析成最终 inode；别名需要分别配置。
- 省略 args 的 path-only 候选不需要为本地匹配读取 argv；存在 args 规则时，路由选择前按配置上限复制 argv，失败按 `failure_decision`。
- 本地 allow/deny 不调用 WASM；gray 复用受限 argv 快照，reusable 缓存键包含 argv digest，不保存明文 argv。
- timeout、插件错误和过载按 gray fallback；目标在决策期间卸载固定 deny。

## 从源码构建

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wit-component/command-policy-dynamic/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_command_policy_dynamic.wasm \
  ../component-command-policy-dynamic.wasm
```

## 文件说明

| 文件 | 说明 |
| --- | --- |
| `plugin.toml` | manifest、runtime-managed config 和 command-policy capabilities。 |
| `command-policy-dynamic.config.json` | 默认空规则配置。 |
| `config.schema.json` | Web/daemon JSON Schema。 |
| `component-command-policy-dynamic.wasm` | 可加载的 WIT 0.4 component。 |
| `fixture-src/` | 插件 Rust 源码。 |
