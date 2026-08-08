# AcTrail 命令安全插件功能体验指南

本文面向具备 Linux 命令行和浏览器操作能力的产品使用者与运维人员，用于部署并体验 AcTrail 的动态命令安全插件：

- `wasm.command-policy-dynamic`：通过 Web 动态维护 executable 与可选参数范围，对 trace 内的命令执行实时放行或拒绝。

体验过程会在默认部署环境中启动真实的 Xiaoo agent，并依次验证插件管理、最小授权、配置原子校验、参数级命令治理、审计事件、越界告警和卸载恢复。

完整体验约需 15～20 分钟。首次操作前建议完整预检一次环境。

## 1. 功能体验流程

建议把整个场景串成一条完整故事：

1. 操作人员在 Web 中加载动态命令策略插件，只授权它发布 `/usr/bin/bash` 的 deny 规则。
2. 一份包含越权 executable 的配置在 Test 阶段被整批拒绝，插件内存配置和 daemon revision 都不变化。
3. 操作人员下发 `args=["-c","*"]` 的有效规则；同一 Bash binary 执行 `--version` 仍然放行。
4. 真实 Xiaoo 使用 Bash tool 创建标记文件时，目标 `execve`/`execveat` 立即收到操作系统拒绝。
5. AcTrail 同时保留 Enforcement 审计事件，并异步生成命令越界告警。
6. 卸载策略 owner 后，再次运行真实 Xiaoo，原命令恢复默认放行并成功创建标记文件。

完成后应得到以下相互印证的结果：

| 阶段 | 操作系统结果 | AcTrail 证据 | 核心含义 |
| --- | --- | --- | --- |
| 参数不匹配 | `/usr/bin/bash --version` 成功 | dry-run 不命中 `[-c,*]` | 规则只治理指定的 `argv[1..]` |
| 策略生效 | Xiaoo 的 Bash tool 返回 `EPERM` | Enforcement + `command.execution.boundary-violation` | 显式 deny 已进入真实执行路径 |
| owner 卸载 | 新的 Xiaoo trace 成功创建标记 | 插件实例不再 active | owner 规则随卸载立即撤销 |

## 2. 开始前必须知道的边界

### 2.1 命令治理只作用于 AcTrail launch trace

普通终端中的 `/usr/bin/bash -c true` 不在本次治理范围内。必须使用 `actrailctl launch` 启动进程，且启动输出必须包含：

```text
deployment_required_capabilities=...,enforcement-command-execution-seccomp
trace trace-N entered Active
```

该 capability 依赖 launch 时安装的 seccomp user-notify filter，不能通过 `track-add` 给已经运行的普通进程补装。capture profile 还必须同时包含 `proc-lifecycle`，否则配置会 fail-fast。

动态规则可以立即影响已经运行、且启动时请求了该 capability 的 trace。本指南主流程使用一次性 Xiaoo CLI，因此卸载后的恢复通过新的真实 Xiaoo trace 验证；不要把它误写成“同一个一次性 agent 会话内继续对话”。

### 2.2 executable 与参数按精确语义匹配

实际规则中的 `executable` 必须是 tracee namespace 内的精确绝对路径。AcTrail 会做词法规范化，但不会把 symlink 解析到最终 inode，因此 `/bin/bash` 和 `/usr/bin/bash` 是不同的规则目标。

`args` 匹配 `argv[1..]`：

- 省略 `args` 表示匹配该 executable 的任意参数；
- `args=[]` 只匹配没有额外参数的调用；
- 只有末尾的 `"*"` 表示任意剩余参数，包括零个；
- 非末尾的 `"*"` 会在配置校验阶段拒绝整份配置；
- 普通字符串中的 `*` 没有通配含义。

因此 `args=["-c","*"]` 会匹配 Bash command mode，但不会匹配 `/usr/bin/bash --version`。

### 2.3 Host grant 是发布上限，不是实际规则

Web 加载时填写的 executable scope 决定插件最多可以发布哪些 decision 和路径。grant 可以是精确绝对路径，也可以是以 `/**` 结尾的目录范围；插件实际发布的每条规则仍然必须指向一个精确 executable。

一次 Configuration Test 或 Update 是全有或全无。任一规则越过 decision/path grant、格式非法、base revision 过期，或引用不可用的 gray target 时，整批拒绝，不能只应用其中合法的部分。

### 2.4 动态规则属于插件实例内存

`wasm.command-policy-dynamic` 的 Web Configuration 和 Plugin command 操作同一份插件内存配置。配置通过校验后由插件发布给 daemon，再与静态规则和其他 owner 的动态规则合并。不要把它讲成 Web 直接修改 daemon 路由表。

daemon 或插件实例重启后，不要假设上一次配置仍然存在。每次启动后都应重新读取 Configuration，确认规则为预期状态。

卸载 publisher 时，daemon 会先撤销该 owner 发布的规则并清理相关缓存，再删除插件 runtime。其他 owner 或静态规则不会随之删除。

### 2.5 拒绝同步生效，告警异步落库

目标 exec 的拒绝是同步结果；相应 Enforcement 审计沿真实 trace 写入，越界告警则通过异步入口写入独立的 `alerts` 表。不能把“Xiaoo 已报告 Permission denied”和“告警已经落库”视为同一时刻。

只有显式本地 deny、gray plugin deny 和 gray cache deny 会产生 `command.execution.boundary-violation`。default、failure 或 gray fallback deny 仍会写 Enforcement 和错误元数据，但不会伪装成用户策略越界告警。

体验时让 Alerts 页面保持打开，由默认 1 秒轮询刷新。前端轮询使用 `/api/alerts`，本文的 curl 示例显式使用 `?limit=20`，用于保留更多排障上下文，因此两者在告警很多时可能显示不同数量。

### 2.6 插件包“已安装”不等于插件“已加载”

安装脚本只把候选包放入 `~/.actrail/plugins`。体验功能前还要在 Web 的 **Plugins** 页面显式加载。候选插件显示为 `Unloaded` 是正常状态。

### 2.7 Web 默认只监听本机

默认地址是 `127.0.0.1:18080`。可在运行 AcTrail 的主机上打开浏览器，或通过 SSH 端口转发访问：

```bash
ssh -L 18080:127.0.0.1:18080 root@192.0.2.10
```

示例中的 `192.0.2.10` 是文档保留地址，执行前替换为运行 AcTrail 主机的 IP 地址或主机名。然后在本机打开 `http://127.0.0.1:18080`。不要为了方便把管理界面直接暴露到公网。

## 3. 环境要求

运行环境必须满足：

- Linux 主机支持 eBPF 和 seccomp user notification；
- 使用 root，或具备安装 seccomp listener、加载 eBPF 等所需的等价权限；
- 已安装 Rust 1.90 或更高版本、Node.js 18 或更高版本及仓库构建依赖；
- Xiaoo 已安装并完成模型提供方配置，能真实响应一次 CLI 请求并使用 Bash tool；
- `/usr/bin/bash` 存在且可执行；如果现场 Bash 是其他绝对路径，必须同步替换本文所有规则、dry-run 和 prompt 中的路径；
- 仓库位于运行主机的本地磁盘；
- 端口 `18080` 未被占用；
- 本次体验使用的默认数据库允许清空。

以下命令假设仓库路径为 `/root/projects/AcTrail`。如果实际路径不同，只需在每个终端进入真实仓库目录；后续命令会通过 `pwd -P` 自动得到绝对路径。本文界面示例以 `/usr/bin/bash` 为准。

## 4. 编译和部署

> 首次体验或源码更新后执行本节。后续重复体验可直接从环境检查开始。

### 4.1 使用一致的运行身份

输入：

```bash
sudo -i
cd /root/projects/AcTrail
export ACTRAIL_REPO="$(pwd -P)"
export DEMO_ROOT="$ACTRAIL_REPO/temp/command-security-experience"
export ACTRAIL_WEB="http://127.0.0.1:18080"
export COMMAND_PLUGIN="wasm.command-policy-dynamic"
export BASH_EXECUTABLE="$(command -v bash)"
export MARKER="$DEMO_ROOT/command-boundary-marker.txt"
```

预期现象：

```bash
id -u
printf '%s\n' "$HOME" "$ACTRAIL_REPO" "$DEMO_ROOT" "$BASH_EXECUTABLE" "$MARKER"
```

应依次看到 root 用户 ID `0`、`/root`、仓库绝对路径、体验目录、`/usr/bin/bash` 和标记文件绝对路径。

必须让构建、插件安装、daemon 和 Web 使用同一个 `HOME`。否则插件可能被安装到一个用户的 `~/.actrail/plugins`，Web 却从另一个用户的目录扫描。

如果 `BASH_EXECUTABLE` 不是 `/usr/bin/bash`，后续 Web 表单中的 executable 和 Xiaoo prompt 也必须使用现场输出的绝对路径。

### 4.2 构建并安装最新程序和插件

输入：

```bash
cargo fmt
scripts/install-release.sh /usr/local/bin
```

预期现象：

- 前端需要重建时，Vite 输出 `built in ...`；
- Cargo 输出 `Finished release profile`；
- 安装器构建 `wasm32-wasip2` 官方插件；
- 最后一行类似：

```text
installed AcTrail binaries to /usr/local/bin and plugins to /root/.actrail/plugins
```

检查输入：

```bash
command -v actraild actrailctl actrailviewer actrailweb xiaoo
find "$HOME/.actrail/plugins/command-policy-dynamic" -maxdepth 1 -type f -print | sort
```

预期现象：

- 四个 AcTrail 程序来自 `/usr/local/bin`；
- Xiaoo 来自现场安装路径；
- `command-policy-dynamic` 包包含 manifest、JSON 配置、JSON Schema 和 Wasm 文件：

```text
command-policy-dynamic.config.json
command-policy-dynamic.plugin.toml
component-command-policy-dynamic.wasm
config.schema.json
```

### 4.3 用默认配置初始化运行环境

> `--force` 会覆盖 `/etc/actrail/actraild.conf`。只在专用体验环境或已确认允许覆盖的环境执行。

如果 `actraild status` 显示正在运行，先输入：

```bash
actraild stop
```

然后输入：

```bash
actraild init --force
```

默认配置已经启用 seccomp notify 和命令治理，并在 capture profile 中请求 `proc-lifecycle` 与 `enforcement-command-execution-seccomp`。核对以下关键值，不需要另建专用配置：

```toml
[seccomp_notify]
enabled = true
reserved_listener_fd = 253

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

检查输入：

```bash
grep -n 'enforcement-command-execution-seccomp' /etc/actrail/actraild.conf
grep -A16 '^\[command_control\]' /etc/actrail/actraild.conf
grep -A4 '^\[command_control\.gray\]' /etc/actrail/actraild.conf
grep -A3 '^\[seccomp_notify\]' /etc/actrail/actraild.conf
```

预期现象：capture capability 存在，`command_control.enabled = true`，`seccomp_notify.enabled = true`，默认 decision 为 allow，failure 和 gray fallback 为 deny。

静态命令规则文件缺失或为空是合法状态；没有插件规则匹配时，默认 `allow` 应直接放行。如果 `/etc/actrail/command-control.rules` 已经存在且包含旧规则，输入：

```bash
sed -n '1,120p' /etc/actrail/command-control.rules
```

停止准备并人工核对。静态规则会与插件规则合并，不应在未经确认时继续体验，也不要直接删除不属于本次体验的规则。

### 4.4 清理上一次体验数据并启动服务

> `actrailctl clean` 会删除默认数据库、daemon 日志和本地运行产物。截图或证据需要保留时，必须先导出再执行。

终端 A 输入：

```bash
actrailctl clean
actraild start
actraild status
actrailctl doctor
```

预期现象：

- `actraild started pid=...`；
- status 显示 `actraild running`；
- doctor 输出中包括 `storage_ready=true`；
- 配置和部署检查没有 command-control 或 seccomp-notify 错误。

终端 B 输入：

```bash
sudo -i
cd /root/projects/AcTrail
actrailweb
```

预期现象：

```text
actrailweb listening on http://127.0.0.1:18080 storage=/var/lib/actrail/actrail.sqlite
actrailweb is running; press Ctrl-C to stop
```

REST 健康检查：

```bash
curl --noproxy '*' -sS -o /dev/null -w 'frontend_http=%{http_code}\n' \
  http://127.0.0.1:18080/
curl --noproxy '*' -sS \
  http://127.0.0.1:18080/api/plugins/catalog
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/alerts?limit=20'
```

预期现象：

- `frontend_http=200`；
- catalog 返回 `"available":true`；当前 release 安装器会安装 6 个官方候选包；
- `command-policy-dynamic` 的 `activation_ready` 为 `true`、`issue` 为 `null`；
- 干净数据库返回 `{"alerts":[]}`。

### 4.5 准备体验目录

终端 C 输入：

```bash
sudo -i
cd /root/projects/AcTrail
export ACTRAIL_REPO="$(pwd -P)"
export DEMO_ROOT="$ACTRAIL_REPO/temp/command-security-experience"
export ACTRAIL_WEB="http://127.0.0.1:18080"
export COMMAND_PLUGIN="wasm.command-policy-dynamic"
export BASH_EXECUTABLE="$(command -v bash)"
export MARKER="$DEMO_ROOT/command-boundary-marker.txt"
mkdir -p "$DEMO_ROOT"
rm -f "$MARKER"
```

检查输入：

```bash
test "$BASH_EXECUTABLE" = /usr/bin/bash
test ! -e "$MARKER"
printf '%s\n' "$BASH_EXECUTABLE" "$MARKER"
```

预期现象：两个 `test` 都成功，随后显示 `/usr/bin/bash` 和仓库 `temp/command-security-experience` 下的标记文件路径。

### 4.6 验证真实 agent 可用

输入：

```bash
xiaoo --cli run -p 'Reply exactly: ACTRAIL_COMMAND_EXPERIENCE_READY'
```

预期现象：Xiaoo 输出 `ACTRAIL_COMMAND_EXPERIENCE_READY`。如果模型认证、网络、配额或工具配置失败，应先修复环境；不要替换成 shell 脚本并把结果视为真实 agent 体验已经完成。

## 5. 建立 Alerts 基线

使用两个浏览器标签页，避免在插件配置和告警观察之间来回切换：

1. 标签页 A 打开 `http://127.0.0.1:18080`，进入 **Stats**。
2. 在 Stats 左侧栏点击 **Alerts**；Stats 初次打开时默认选中的是 **LLM Requests**。
3. 确认自动刷新间隔为 `1` 秒。
4. 确认页面显示 0 条告警，然后让标签页 A 始终停留在 Alerts 页面。
5. 复制标签页得到标签页 B，标签页 B 专门用于 **Plugins** 操作。

两个标签页分别维护自己的前端状态。标签页 A 首次进入 Alerts 页面只建立已见告警基线，不弹出“新增 0 条”或历史告警数量 Toast。

REST 验证：

```bash
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/alerts?limit=20'
```

预期返回：

```json
{"alerts":[]}
```

## 6. 场景：动态命令治理、越界告警和卸载恢复

### 6.1 工作原理

> 插件把规则保存在自己的内存配置中。Web 配置先提交给插件，插件验证通过后把自己的规则发布给 daemon；daemon 再与静态规则和其他 owner 的路由合并。trace 启动时安装的 seccomp filter 捕获 `execve`/`execveat`，daemon 按 executable 和 `argv[1..]` 做实际决策。

本场景只授权插件管理 `/usr/bin/bash` 的 deny 规则，体现最小权限。有效规则拒绝 Bash command mode，但不拒绝同一 binary 的其他参数。

### 6.2 在 Web 加载动态策略插件并授予最小范围

界面操作：

1. 在标签页 B 进入 **Plugins**。
2. 点击 **Refresh**。
3. 在 **Plugin candidates** 找到 `wasm.command-policy-dynamic`。
4. 展开候选，确认：
   - Plugin ID 是 `wasm.command-policy-dynamic`；
   - purpose 的接口值是 `control-decider`，UI 标签显示为 `controller`；
   - Built-in access 包含 `command-policy.rules.read`、`command-policy.rules.match-dry-run` 和 `command-policy.rules.validate`；
   - `Issue` 为 `none`。
5. 点击右侧 `Unloaded / Configure & load`。
6. Runtime instance name 保持 `wasm.command-policy-dynamic`。
7. 在 **Executables this plugin can manage** 的 Executable scope 填入：

```text
/usr/bin/bash
```

8. Rule types 只保留 **Deny**；取消 **Allow** 和 **Ask plugin**。
9. 点击 **Load plugin**。

预期现象：

- 候选从上方列表消失；
- `wasm.command-policy-dynamic` 出现在 **Loaded plugin instances**；
- 右侧状态显示 `Active / Unload plugin`；
- Instance ID 和 Plugin ID 分别清楚显示；
- Host grants 包含三项只读/校验能力，以及：

```text
command-policy.rules.apply:kind=deny,path=/usr/bin/bash
```

- Last error 为 `none`。

加载操作的 REST 等价命令如下。Web 已加载成功后不要重复执行 POST；此命令用于无浏览器彩排或排障：

```bash
curl --noproxy '*' -sS -X POST \
  "$ACTRAIL_WEB/api/plugins/catalog/load?package=command-policy-dynamic" \
  -H 'Content-Type: application/json' \
  --data "{\"instance_id\":\"$COMMAND_PLUGIN\",\"grants\":{\"command_policy_rules_apply\":[{\"decision\":\"deny\",\"path_scope\":\"$BASH_EXECUTABLE\"}]}}"
```

Web 操作后的 REST 验证：

```bash
curl --noproxy '*' -sS \
  "$ACTRAIL_WEB/api/plugins/catalog"
curl --noproxy '*' -sS \
  "$ACTRAIL_WEB/api/plugins/runtime/config?instance_id=$COMMAND_PLUGIN"
```

`GET /api/plugins/catalog` 的 `runtime_plugins[]` 中应包含以下关键字段：

```json
{
  "instance_id": "wasm.command-policy-dynamic",
  "plugin_id": "wasm.command-policy-dynamic",
  "state": "active"
}
```

`GET /api/plugins/runtime/config` 的响应中应看到：

```json
{
  "available": true,
  "instance_id": "wasm.command-policy-dynamic",
  "plugin_id": "wasm.command-policy-dynamic",
  "editable": true,
  "config": {
    "rules": []
  },
  "schema": {
    "title": "Dynamic command policy",
    "type": "object"
  }
}
```

实际 `schema` 还包含每个字段的类型、约束、枚举和说明，前端根据这份完整 Schema 渲染控件。

### 6.3 证明越权配置被原子拒绝

这个步骤用于体验 Host grant 和 all-or-nothing 边界。

先记录 daemon 当前 revision。展开 **Plugin command**，每行输入一个参数：

```text
rule
dry-run
/usr/bin/bash
--args-json
["-c","probe"]
```

点击 **Send command**。初始 stdout 应包含：

```text
matched=false decision=allow rule_id=none owner=none executable=/usr/bin/bash rule_revision=none source_revision=...
```

记下 `source_revision`。

然后展开 **Configuration**，添加两条草稿：

```json
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
```

点击 **Test configuration**。

预期现象：

- 页面显示配置错误；
- **Update configuration** 保持不可用；
- 错误指出插件缺少 `/srv/not-granted-command` 的 deny apply grant；
- 第一条合法规则也没有被局部应用；
- 插件当前内存配置仍然是 `{"rules":[]}`；
- 再次执行前面的 dry-run，`source_revision` 与测试前相同。

REST 校验等价命令：

```bash
curl --noproxy '*' -sS -X POST \
  "$ACTRAIL_WEB/api/plugins/runtime/config/validate?instance_id=$COMMAND_PLUGIN" \
  -H 'Content-Type: application/json' \
  --data "{\"config\":{\"rules\":[{\"decision\":\"deny\",\"executable\":\"$BASH_EXECUTABLE\",\"args\":[\"-c\",\"*\"],\"priority\":20},{\"decision\":\"deny\",\"executable\":\"/srv/not-granted-command\",\"priority\":10}]}}"
```

预期返回关键内容：

```json
{
  "valid": false,
  "errors": [
    "rules[1]: missing command-policy.rules.apply grant for deny /srv/not-granted-command"
  ]
}
```

错误数组的索引前缀以现场版本为准，但必须包含 `missing command-policy.rules.apply grant for deny /srv/not-granted-command`。

删除越权草稿后再继续；不要在错误草稿上直接猜测性修改并跳过重新测试。

### 6.4 通过 Configuration 下发有效参数规则

界面操作：

1. 在 **Command execution routes** 中只保留一条 entry。
2. Decision 从下拉选项选择 **deny**。
3. Executable 填 `/usr/bin/bash`。
4. 在 Arguments 中依次添加 `-c` 和 `*` 两项。
5. Priority 填 `20`；默认值是 `10`，这里显式改为 `20` 便于识别。
6. Rule ID 留空，让插件生成稳定 ID。
7. Gray target instance ID 留空；它只用于 **gray** 决策。
8. 点击 **Test configuration**。
9. 看到 `Test passed — ready to update` 后，点击 **Update configuration**。

测试结果绑定到测试时的完整草稿。测试成功后只要再修改任一字段，**Update configuration** 就会重新禁用，必须再次点击 **Test configuration**。

预期现象：

- 更新前必须测试成功，Update 按钮才可用；
- 更新成功提示 `Runtime configuration updated.`；
- Rule ID 自动变成 `command-dynamic-1`；
- Decision 是选择控件，不是自由文本输入；
- 页面重新读取后仍显示 `args=["-c","*"]`。

如果不使用 Web，等价 REST 操作分两步。Web 已经更新后不要再执行 POST 更新：

```bash
curl --noproxy '*' -sS -X POST \
  "$ACTRAIL_WEB/api/plugins/runtime/config/validate?instance_id=$COMMAND_PLUGIN" \
  -H 'Content-Type: application/json' \
  --data "{\"config\":{\"rules\":[{\"decision\":\"deny\",\"executable\":\"$BASH_EXECUTABLE\",\"args\":[\"-c\",\"*\"],\"priority\":20}]}}"

curl --noproxy '*' -sS -X POST \
  "$ACTRAIL_WEB/api/plugins/runtime/config?instance_id=$COMMAND_PLUGIN" \
  -H 'Content-Type: application/json' \
  --data "{\"config\":{\"rules\":[{\"decision\":\"deny\",\"executable\":\"$BASH_EXECUTABLE\",\"args\":[\"-c\",\"*\"],\"priority\":20}]}}"
```

预期校验返回 `"valid":true`，更新响应的 rule 包含 `"rule_id":"command-dynamic-1"`。

Web 操作后的只读 REST 验证：

```bash
curl --noproxy '*' -sS \
  "$ACTRAIL_WEB/api/plugins/runtime/config?instance_id=$COMMAND_PLUGIN"
```

### 6.5 用 Plugin command 查询实际合并命中

展开 **Plugin command**，每行输入一个参数：

```text
rule
dry-run
/usr/bin/bash
--args-json
["-c","printf test"]
```

点击 **Send command**。

预期 stdout 包含：

```text
matched=true decision=deny rule_id=command-dynamic-1 owner=wasm.command-policy-dynamic executable=/usr/bin/bash rule_revision=... source_revision=...
```

这个命令只查询 daemon 当前合并后的实际路由，不修改配置。命令成功后 Web 会重新读取插件配置，配置内容应保持不变。

REST 等价命令：

```bash
curl --noproxy '*' -sS -X POST \
  "$ACTRAIL_WEB/api/plugins/runtime/command?instance_id=$COMMAND_PLUGIN" \
  -H 'Content-Type: application/json' \
  --data "{\"argv\":[\"rule\",\"dry-run\",\"$BASH_EXECUTABLE\",\"--args-json\",\"[\\\"-c\\\",\\\"printf test\\\"]\"]}"
```

再读取一次配置，证明只读命令没有修改插件内存：

```bash
curl --noproxy '*' -sS \
  "$ACTRAIL_WEB/api/plugins/runtime/config?instance_id=$COMMAND_PLUGIN"
```

### 6.6 验证同一 executable 的非匹配参数仍然放行

终端 C 输入：

```bash
actrailctl launch --name command-policy-argv-control -- \
  "$BASH_EXECUTABLE" --version
```

预期现象：

- trace 进入 Active；
- 输出包含 `GNU bash` 版本信息；
- launch 成功结束。

该命令使用同一个 `/usr/bin/bash`，但 `argv[1..]=["--version"]` 不匹配 `args=["-c","*"]`，因此走 `default_decision = "allow"`。默认配置的 `audit_default_allow = false`，所以不要求为这次普通放行产生 Enforcement 事件。

### 6.7 运行真实 Xiaoo 触发同步拒绝

确保标签页 A 仍停留在 **Stats → Alerts**，并确认标记文件不存在：

```bash
rm -f "$MARKER"
test ! -e "$MARKER"
```

然后输入：

```bash
actrailctl launch --name command-policy-xiaoo-denied -- \
  xiaoo --cli run --tools bash --max-turns 3 -p \
  "Use the Bash tool exactly once to write ACTRAIL_COMMAND_OK to $MARKER. Report the exact operating-system result. Do not use another shell, programming language, or file-writing tool."
```

预期终端现象：

- 新 trace 进入 Active，例如 `trace trace-2 entered Active`；
- launch 输出包含 `enforcement-command-execution-seccomp`；
- Xiaoo 的 Bash tool 报告 `Permission denied`、`Operation not permitted` 或等价的 `EPERM`；
- agent 本身仍可完成并如实汇报失败。

检查标记：

```bash
test ! -e "$MARKER"
```

该检查必须成功。操作系统拒绝和标记不存在是同步治理结果；不要只根据 Xiaoo 的自然语言总结判断策略是否生效。

### 6.8 展示 Enforcement 审计证据

把 launch 输出中的数字 `N` 记录为现场 trace ID，例如：

```bash
export TRACE_ID=2
actrailviewer events --trace-id "$TRACE_ID" | \
  grep -E 'Enforcement|seccomp-user-notify|execve|execveat|command-dynamic-1|/usr/bin/bash|denied'
```

预期至少看到一条包含以下语义的 Enforcement 事件：

```text
Enforcement ... operation=execve|execveat decision=deny path=/usr/bin/bash result=denied backend=seccomp-user-notify rule_id=command-dynamic-1 ...
```

实际字段顺序以现场输出为准。还应能确认：

- policy owner 是 `wasm.command-policy-dynamic`；
- decision source 对本地显式 deny 是 fast-path/rule 语义；
- 没有把 default、failure 或 fallback deny 误写成显式规则命中。

这证明“操作系统结果”“治理审计”和“告警”是三个可以相互印证、但职责不同的结果。

### 6.9 在 Alerts 页面展示异步越界告警

观察标签页 A 中保持打开的 **Stats → Alerts**。

预期现象：

- 新告警从列表顶部滑入；
- 页面 Toast 提示 `新增 1 条告警`；
- 告警标题为 `Out-of-bound command execution denied`；当前 daemon 内置告警标题使用英文；
- kind 为 `command.execution.boundary-violation`；
- severity 为 `high`；
- producer 是 `actraild.enforcement`；
- payload 中显示 `/usr/bin/bash`、`command-dynamic-1`、`wasm.command-policy-dynamic` 和 `fast-path-deny`。

告警没有立即出现时先等待自动轮询，不要重新运行 agent。也可以点击 **Refresh**，或重复以下只读请求：

```bash
curl --noproxy '*' -sS \
  "$ACTRAIL_WEB/api/alerts?limit=20"
curl --noproxy '*' -sS \
  "$ACTRAIL_WEB/api/traces/$TRACE_ID/alerts?limit=20"
```

预期最新一条告警包含：

```json
{
  "producer_plugin_id": "actraild.enforcement",
  "definition_key": "command-execution-boundary-violation",
  "kind": "command.execution.boundary-violation",
  "severity": "high",
  "payload": {
    "decision_source": "fast-path-deny",
    "executable": "/usr/bin/bash",
    "operation": "execve",
    "policy_owner_instance_id": "wasm.command-policy-dynamic",
    "process_id": 3,
    "rule_id": "command-dynamic-1"
  }
}
```

`operation` 也可能是 `execveat`；`process_id` 是该 trace 内触发执行的 AcTrail 进程 ID，具体数字以现场响应为准，不是操作系统 PID。

### 6.10 卸载 owner 后用真实 Xiaoo 验证恢复

在标签页 B 中找到 `wasm.command-policy-dynamic`，点击右侧 active 控件，再点击 **Unload plugin**。

预期现象：

- Loaded plugin instances 中不再显示该实例；
- 包重新出现在 Plugin candidates；
- 该 owner 发布的 `command-dynamic-1` 和相关缓存立即从 daemon 合并路由中撤销。

REST 等价卸载命令。如果已经通过 Web 卸载，不要重复执行：

```bash
curl --noproxy '*' -sS -X POST \
  "$ACTRAIL_WEB/api/plugins/runtime/unload?instance_id=$COMMAND_PLUGIN"
```

只读验证：

```bash
curl --noproxy '*' -sS \
  "$ACTRAIL_WEB/api/plugins/catalog"
```

如果体验前没有其他 active 插件，预期 `runtime_plugin_count` 为 `0`，`command-policy-dynamic` 的 `loaded_instances` 为空。

保持标记不存在，然后再次运行真实 Xiaoo：

```bash
rm -f "$MARKER"
actrailctl launch --name command-policy-xiaoo-owner-unloaded -- \
  xiaoo --cli run --tools bash --max-turns 3 -p \
  "Use the Bash tool exactly once to write ACTRAIL_COMMAND_OK to $MARKER. Then read the file and report its exact content. Do not use another shell, programming language, or file-writing tool."
```

预期现象：

- 新 trace 进入 Active；
- Xiaoo 的 Bash tool 不再收到权限拒绝；
- Xiaoo 报告文件内容是 `ACTRAIL_COMMAND_OK`。

检查输入：

```bash
test "$(sed -n '1p' "$MARKER")" = ACTRAIL_COMMAND_OK
```

该检查必须成功。这证明插件 owner 卸载后，其动态规则已经撤销。它不证明系统中不存在其他 owner 或静态规则；若现场仍有其他命中规则，应按合并策略解释结果。

## 7. gray 与 reusable 的扩展体验边界

本指南主流程只验证本地 deny，不加载远程决策插件。需要体验 gray 时，必须额外授予 **Ask plugin**，并把 `gray_target` 指向一个 active、非自身的 `control-decider` 实例：

```json
{
  "decision": "gray",
  "executable": "/usr/bin/python3",
  "priority": 10,
  "gray_target": "remote-command-decider"
}
```

`wasm.command-policy-dynamic` 只是策略 publisher，不是 gray 决策器，不能把自身实例 ID 填为 `gray_target`。

完整 gray 验收还应覆盖：

- plugin allow 和 plugin deny；
- 相同 trace、进程 generation、规则 revision、路径和 argv digest 下的 reusable cache 命中；
- 任一 argv 改变后重新调用 decider；
- timeout、plugin error/panic、全局过载、规则过载和实例过载按 `[command_control.gray].fallback` 处理；
- 目标在决策期间卸载时固定 deny；
- 审计 decision source 区分 `gray-plugin` 与 `gray-plugin-cache`；
- reusable cache 不保存明文 argv。

gray 涉及额外插件和同步时序，不应在没有真实 control-decider 与完整证据时标记为本指南主流程已完成。

## 8. 收尾和恢复环境

确认拒绝、告警、卸载恢复三组证据后，在终端 B 按 `Ctrl-C` 停止 Web。终端 A 输入：

```bash
actraild stop
actrailctl clean
rm -rf /root/projects/AcTrail/temp/command-security-experience
```

如果仓库不在 `/root/projects/AcTrail`，把最后一项替换为终端中 `printf '%s\n' "$DEMO_ROOT"` 显示的明确路径，确认无误后再删除。

预期现象：daemon 停止，默认数据库和日志被清理，体验目录被删除。`~/.actrail/plugins` 中的候选包会保留，供下次使用。

## 9. 故障排查顺序

必须按实际运行路径排查，不要先猜 UI 或插件内部问题。

### 9.1 Web 看不到候选插件

输入：

```bash
printf '%s\n' "$HOME"
find "$HOME/.actrail/plugins/command-policy-dynamic" -maxdepth 1 -type f -print | sort
curl --noproxy '*' -sS http://127.0.0.1:18080/api/plugins/catalog
```

检查 catalog 中的 `directory`、`activation_ready`、`issue` 和 `warnings`。最常见原因是安装和运行 Web 使用了不同用户的 `HOME`。

### 9.2 daemon 启动时报告 command-control 配置错误

依次核对：

1. `[command_control].enabled = true`；
2. `[seccomp_notify].enabled = true`；
3. capture 同时包含 `proc-lifecycle` 和 `enforcement-command-execution-seccomp`；
4. 所有路径、argv、pending、cache、timeout 和 concurrency 限制都大于零；
5. `/etc/actrail/command-control.rules` 中没有旧语法或非法规则。

规则文件缺失或为空是合法状态。若日志仍因文件缺失失败，先核对 `/usr/local/bin/actraild` 是否是当前源码构建的 release 产物，不要用伪造 bootstrap 规则绕过旧二进制。

### 9.3 动态插件加载按钮不可用

确认：

- Executable scope 是绝对路径，目录递归范围以 `/**` 结尾；
- 每个范围至少选择一种 Rule type；
- instance name 无首尾空格；
- catalog 中 `activation_ready=true`。

### 9.4 有效配置 Test 失败

依次检查：

1. decision 是否在加载时授予；
2. executable 是否被精确 grant 或 `/**` grant 覆盖；
3. `"*"` 是否只出现在 args 最后一项；
4. 同一 owner 中 rule ID 是否重复；
5. 同一 owner 中 `(executable, args 逻辑范围)` 是否重复；
6. allow/deny 是否错误填写 `gray_target`；
7. gray target 是否 active、是否为其他 control-decider 实例。

Test 失败后必须重新 GET Configuration 和 dry-run revision，确认旧状态未变；不要假设合法子集已经生效。

### 9.5 Bash command mode 没有被拒绝

依次检查：

1. `actrailctl launch` 输出是否请求了 `enforcement-command-execution-seccomp`；
2. 插件是否 active；
3. Configuration 是否确实包含 `command-dynamic-1`；
4. `rule dry-run` 是否返回 `matched=true decision=deny`；
5. 实际 executable 是 `/usr/bin/bash` 还是 `/bin/bash`；
6. 实际 `argv[1..]` 是否以 `-c` 开头；
7. `actrailviewer events` 是否存在相应 seccomp-user-notify Enforcement。

直接在普通 shell 中运行 Bash 不能验证 trace-scoped command enforcement。

### 9.6 `/usr/bin/bash --version` 也被拒绝

输入：

```bash
curl --noproxy '*' -sS \
  "$ACTRAIL_WEB/api/plugins/runtime/config?instance_id=$COMMAND_PLUGIN"
actrailviewer events --trace-id "$TRACE_ID" | grep Enforcement
```

确认规则不是省略 `args`，而是精确的 `["-c","*"]`；同时检查静态规则和其他 owner 是否对 Bash 发布了更高优先级 deny。路径或 argv 捕获失败会使用 `failure_decision`，也需要根据 Enforcement 中的 `command_control_error` 排查。

### 9.7 Xiaoo 报告拒绝但 marker 仍然存在

先确认 marker 是否来自上一次体验：

```bash
rm -f "$MARKER"
test ! -e "$MARKER"
```

重新运行一次后再检查。如果仍然出现，核对 Xiaoo 是否改用了其他 shell、编程语言或文件写入工具，以及被拒绝的 Enforcement 是否真的属于创建 marker 的那次 exec。自然语言报告不能替代文件状态和审计证据。

### 9.8 Enforcement 存在但告警没有出现

依次检查：

1. Enforcement 是否命中显式规则，而不是 default/failure/fallback deny；
2. `rule_id` 和 policy owner 是否存在；
3. Alerts 页面是否仍在轮询；
4. `/api/traces/$TRACE_ID/alerts?limit=20` 是否已经返回告警；
5. daemon 日志是否显示 alert queue admission 或 storage 错误。

告警异步落库。一次没有立即返回不代表不会产生告警，但持续缺失时也不能只靠等待掩盖 admission/storage 故障。

### 9.9 卸载后仍然拒绝

确认 catalog 中实例已经不再 active，再执行 dry-run 或查看新 trace 的 Enforcement。最常见原因是：

- 静态规则仍然命中；
- 另一个动态 owner 发布了相同 executable 范围；
- 实际失败来自 command-control failure decision，而不是已卸载规则；
- 使用的是卸载前已经输出的旧证据，而不是新的 trace。

## 10. 当前体验需要保留的风险认知

1. **动态配置不能视为跨重启持久化状态。** 插件或 daemon 重启后必须重新读取并确认规则，不要依赖上一次操作残留状态。
2. **executable 别名不会自动合并。** `/bin/bash` 与 `/usr/bin/bash` 需要分别授权和配置；验收前必须核对真实运行路径。
3. **一次性 Xiaoo CLI 不证明同 trace 的交互式热更新。** 动态路由能影响已安装 seccomp filter 的活动 trace，但本指南主流程用新 trace 验证卸载恢复；需要同 trace 证据时应使用真正保持活动并可继续发命令的 agent 会话。
4. **真实 agent 输出存在外部依赖。** 模型认证、网络、配额和 agent 自身工具选择都会影响耗时；开始前必须执行 4.6。
5. **告警到达时间不能与 agent 终态绑定。** 前端轮询和手动 REST 查询只能观察最终落库结果，不能把一次没有立即返回当成“不会产生告警”。
6. **不同内核和 libc 的拒绝文案可能不同。** 验收依据应是命令失败、marker 不存在、Enforcement deny 和 boundary alert 四者一致，而不是绑定一条英文错误文本。
7. **gray 不是本地 deny 的自动扩展。** 没有真实 control-decider、timeout/overload 测试和 cache 证据时，不得宣称 gray 流程已经验收。
8. **浏览器视觉仍需按现场尺寸复查。** REST 和真实 agent 证据不能替代实际浏览器中对表单对齐、溢出和交互状态的人工确认。

## 11. 自动化真实 Agent 回归与完成口径

仓库提供与本指南核心路径对应的[真实 Xiaoo 动态命令策略回归](../../../../tests/v2/regression/command_policy_xiaoo/README.zh.md)。它使用真实 Xiaoo、真实 Bash tool、真实 `execve`/seccomp notification、SQLite 审计和 Web 告警，不用 shell 脚本伪造 agent 或治理事件。

从仓库根目录运行：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case command_policy_xiaoo
```

也可以独立运行：

```bash
sudo -E python3.11 \
  tests/v2/regression/command_policy_xiaoo/run_e2e.py
```

成功结果应覆盖：

- Web load 生成 `/usr/bin/bash` 的最小 deny grant；
- 包含越权规则的 Configuration Test 原子拒绝，插件配置和 daemon revision 不变；
- Update 生成稳定 ID `command-dynamic-1`，dry-run 返回 owner、decision 和 revision；
- `/usr/bin/bash --version` 在 `[-c,*]` 规则下保持允许；
- 真实 Xiaoo Bash tool 返回 `EPERM`，marker 不存在；
- trace 中存在 `backend=seccomp-user-notify` 的 Enforcement；
- Web alerts 中存在 high severity 的 `command.execution.boundary-violation`；
- Web 卸载 owner 后，第二次真实 Xiaoo Bash tool 成功并创建 marker。

验收记录至少保存：

- release build 标识和 operator 配置摘要；
- Web load、Test 和 Update 响应；
- launch capability 输出；
- Xiaoo 的原始输出；
- deny 阶段 marker 不存在、卸载后 marker 内容正确的检查；
- Enforcement 和 alert 查询结果；
- owner 卸载后的 catalog 状态。

缺少真实 Xiaoo 运行或上述任一关键证据时，不得把真实 Agent 验收标记为完成，只能记录为待验收。
