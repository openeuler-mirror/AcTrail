# AcTrail 文件安全插件功能体验指南

本文面向具备 Linux 命令行和浏览器操作能力的产品使用者与运维人员，用于部署并体验 AcTrail 的两个文件安全插件：

- `actrail.file-leakage`：在 trace 结束后异步发现仍残留在工作目录之外的泄露文件；
- `wasm.file-policy-dynamic`：通过 Web 动态维护文件访问规则，对 trace 内的文件访问实时放行或拒绝。

体验过程会在默认部署环境中启动真实的 Xiaoo agent，并依次验证插件管理、配置校验、实时治理、审计事件和跨 trace 告警。

完整体验约需 15～20 分钟。首次操作前建议完整预检一次环境。

## 1. 功能体验流程

建议把两个场景串成一条完整故事：

1. agent 成功写出了工作目录外的文件。AcTrail 不阻塞业务，但在 trace 结束后异步生成文件泄露告警。
2. 操作人员通过 Web 为同一类 agent 动态下发精确的文件拒绝规则，无需重启 daemon。
3. agent 仍能读取普通文件，但读取受保护文件时立即收到操作系统拒绝。
4. AcTrail 同时保留 enforcement 审计事件，并异步生成越界访问告警。

完成后应看到两条按时间倒序排列的告警：

| 顺序 | 告警类型 | 生产者 | 严重级别 | 核心证据 |
| --- | --- | --- | --- | --- |
| 新 | `file.access.boundary-violation` | `actraild.enforcement` | `high` | 被拒绝路径、规则 ID、策略所有者、判定来源 |
| 旧 | `file.leakage` | `actrail.file-leakage` | `medium` | trace 结束后仍存在的越界文件路径 |

## 2. 开始前必须知道的边界

### 2.1 告警是异步结果

`file.leakage` 会在 trace 进入终态后开始计算，然后通过异步入口写入独立的 `alerts` 表。告警可能晚于 trace 结束很久才出现，不能把“agent 已退出”和“告警已经落库”视为同一时刻。

越界访问的拒绝是同步生效的，但相应告警仍是异步写入。体验时让 Alerts 页面保持打开，由默认 1 秒轮询刷新；不要把 agent 退出的一瞬间视为告警已经落库。

前端轮询请求使用 `/api/alerts`，由 `[web.alerts]` 的服务器默认限制决定返回条数。本文的 curl 示例显式使用 `?limit=20`，用于保留更多排障上下文，因此两者在告警很多时可能显示不同数量。

### 2.2 动态策略只治理 AcTrail 启动的 trace

普通终端中的 `cat private.txt` 不在本次治理范围内。必须使用 `actrailctl launch` 启动真实 agent，且启动输出必须包含：

```text
deployment_required_capabilities=...,enforcement-file-permission-fanotify
trace trace-N entered Active
```

### 2.3 插件包“已安装”不等于插件“已加载”

安装脚本只把候选包放入 `~/.actrail/plugins`。体验功能前还要在 Web 的 **Plugins** 页面显式加载。候选插件显示为 `Unloaded` 是正常状态。

### 2.4 动态规则属于插件实例内存

`wasm.file-policy-dynamic` 的 Web Configuration 和 Plugin command 操作的是同一份插件内存配置。配置修改后由插件发布给 daemon，再与其他路由合并。不要把它讲成 Web 直接修改 daemon 路由表。

daemon 或插件实例重启后，不要假设上一次配置仍然存在。每次启动后都应重新读取 Configuration，确认规则为预期状态。

### 2.5 Web 默认只监听本机

默认地址是 `127.0.0.1:18080`。可在运行 AcTrail 的主机上打开浏览器，或通过 SSH 端口转发访问：

```bash
ssh -L 18080:127.0.0.1:18080 root@192.0.2.10
```

示例中的 `192.0.2.10` 是文档保留地址，执行前替换为运行 AcTrail 主机的 IP 地址或主机名。然后在本机打开 `http://127.0.0.1:18080`。不要为了方便把管理界面直接暴露到公网。

## 3. 环境要求

运行环境必须满足：

- Linux 主机支持 eBPF、fanotify permission events 和 seccomp notify；
- 使用 root，或具备等价的 `CAP_SYS_ADMIN` 等权限；
- 已安装 Rust 1.90 或更高版本、Node.js 18 或更高版本及仓库构建依赖；
- Xiaoo 已安装并完成模型提供方配置，能真实响应一次 CLI 请求；
- 仓库位于运行主机的本地磁盘；
- 端口 `18080` 未被占用；
- 本次体验使用的默认数据库允许清空。

以下命令假设仓库路径为 `/root/projects/AcTrail`。如果实际路径不同，只需在每个终端进入真实仓库目录；后续命令会通过 `pwd -P` 自动得到绝对路径。

## 4. 编译和部署

> 首次体验或源码更新后执行本节。后续重复体验可直接从环境检查开始。

### 4.1 使用一致的运行身份

输入：

```bash
sudo -i
cd /root/projects/AcTrail
export ACTRAIL_REPO="$(pwd -P)"
export DEMO_ROOT="$ACTRAIL_REPO/temp/file-security-experience"
```

预期现象：

```bash
id -u
printf '%s\n' "$HOME" "$ACTRAIL_REPO" "$DEMO_ROOT"
```

应依次看到 root 用户 ID `0`、`/root`、仓库绝对路径和 `temp/file-security-experience` 路径。

必须让构建、插件安装、daemon 和 Web 使用同一个 `HOME`。否则插件可能被安装到一个用户的 `~/.actrail/plugins`，Web 却从另一个用户的目录扫描。

### 4.2 显式构建最新主程序和前端

输入：

```bash
scripts/install-build-deps.sh --install
npm --prefix crates/apps/web/frontend run build
cargo fmt
cargo build --workspace --release
scripts/install-release.sh /usr/local/bin
```

预期现象：

- Vite 输出 `built in ...`；
- Cargo 输出 `Finished release profile`；
- 安装器构建两个 `wasm32-wasip2` 插件；
- 最后一行类似：

```text
installed AcTrail binaries to /usr/local/bin and plugins to /root/.actrail/plugins
```

不要只执行 `scripts/install-release.sh`。当前安装器在 `target/release` 中已有可执行文件时不会主动判断源代码是否更新；显式执行 release build 可以避免把旧二进制部署到 `/usr/local/bin`。

检查输入：

```bash
command -v actraild actrailctl actrailviewer actrailweb xiaoo
find "$HOME/.actrail/plugins" -maxdepth 2 -type f -print | sort
```

预期现象：

- 四个 AcTrail 程序来自 `/usr/local/bin`；
- 能找到 `file-leakage` 和 `file-policy-dynamic` 两个完整插件包；
- 两个包都包含 manifest、JSON 配置、JSON Schema 和 Wasm 文件。

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

默认配置已经启用 fanotify enforcement、seccomp user notification，以及 `mkdir`/`rmdir` 目录操作控制。核对以下默认值，不需要另建专用配置：

```toml
[enforcement]
enabled = true
backend = "fanotify"
scope = "trace"
rules_path = "/etc/actrail/enforcement-rules.conf"
builtin_rules = []
default_decision = "allow"
mark_strategy = "parent-directories"
audit_enabled = true
event_buffer_bytes = 65536
seccomp_syscalls = ["mkdir", "rmdir"]
seccomp_path_max_bytes = 4096
```

不要创建 bootstrap enforcement 规则文件。静态规则文件缺失或为空是合法状态；没有插件规则匹配时，默认 `allow` 应直接放行。

检查输入：

```bash
grep -n 'enforcement-file-permission-fanotify' /etc/actrail/actraild.conf
grep -A12 '^\[enforcement\]' /etc/actrail/actraild.conf
```

预期现象：capture capability 存在，`enabled = true`，`default_decision = "allow"`，目录操作列表包含 `mkdir` 和 `rmdir`。

如果 `/etc/actrail/enforcement-rules.conf` 已经存在且包含旧规则，停止准备并人工核对；旧静态规则会与插件规则合并，不应在未经确认时继续体验。

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
- collectors 中包括 `fanotify-enforcement`。

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
- catalog 返回 `"available":true`、`"package_count":2`；
- 两个 package 的 `activation_ready` 都是 `true`；
- 干净数据库返回 `{"alerts":[]}`。

### 4.5 准备体验数据

终端 C 输入：

```bash
sudo -i
cd /root/projects/AcTrail
export ACTRAIL_REPO="$(pwd -P)"
export DEMO_ROOT="$ACTRAIL_REPO/temp/file-security-experience"
mkdir -p "$DEMO_ROOT/work" "$DEMO_ROOT/outside"
printf 'public file access example\n' > "$DEMO_ROOT/work/public.txt"
printf 'confidential file access example\n' > "$DEMO_ROOT/work/private.txt"
rm -f "$DEMO_ROOT/outside/leaked-data.txt"
```

检查输入：

```bash
find "$DEMO_ROOT" -maxdepth 2 -type f -print -exec sed -n '1p' {} \;
```

预期现象：只看到 `public.txt` 和 `private.txt`；泄露目标文件尚不存在。

### 4.6 验证真实 agent 可用

输入：

```bash
xiaoo --cli run -p 'Reply exactly: ACTRAIL_EXPERIENCE_READY'
```

预期现象：Xiaoo 输出 `ACTRAIL_EXPERIENCE_READY`。如果模型认证、网络或配额失败，应先修复环境；不要替换成 shell 脚本并把结果视为真实 agent 体验已经完成。

## 5. 建立 Alerts 基线

使用两个浏览器标签页，避免在插件配置和告警观察之间来回切换：

1. 标签页 A 打开 `http://127.0.0.1:18080`，进入 **Stats**。
2. 在 Stats 左侧栏点击 **Alerts**；Stats 初次打开时默认选中的是 **LLM Requests**。
3. 确认自动刷新间隔为 `1` 秒。
4. 确认页面显示 0 条告警，然后让标签页 A 始终停留在 Alerts 页面。
5. 复制标签页得到标签页 B，标签页 B 专门用于 **Plugins** 操作。

两个标签页分别维护自己的前端状态，不共享 Alerts 基线。标签页 A 首次进入 Alerts 页面只建立已见告警基线，不弹出“新增 0 条”或历史告警数量 Toast；以后没有新增告警时重新进入也不会显示零计数 Toast。

REST 验证：

```bash
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/alerts?limit=20'
```

预期返回：

```json
{"alerts":[]}
```

## 6. 场景一：文件泄露异步告警

### 6.1 工作原理

> 第一个插件只观察成功的文件写操作，不在写入热路径上同步做泄露计算。它异步保存越界候选，在 trace 结束事件到达后检查候选文件是否仍存在，再异步写入独立的告警表。

### 6.2 在 Web 加载 file-leakage

界面操作：

1. 在标签页 B 进入 **Plugins**。
2. 点击 **Refresh**。
3. 在 **Plugin candidates** 找到 `actrail.file-leakage`。
4. 展开候选，确认：
   - Plugin ID 是 `actrail.file-leakage`；
   - purpose 的接口值是 `observation-consumer`，UI 标签显示为 `observer`；
   - Built-in access 包含 `trace-file-state-read` 和 `alert-write`；
   - `Issue` 为 `none`。
5. 点击右侧 `Unloaded / Load plugin` 控件。
6. Runtime instance name 保持 `actrail.file-leakage`。
7. 点击 **Load plugin**。

预期现象：

- 候选从上方列表消失；
- `actrail.file-leakage` 出现在 **Loaded plugin instances**；
- 右侧状态显示 `Active / Unload plugin`；
- Instance ID 和 Plugin ID 分别清楚显示；
- Host grants 为 `trace-file-state-read`、`alert-write`；
- Last error 为 `none`。

加载操作的 REST 等价命令如下。Web 已加载成功后不要重复执行 POST；此命令用于无浏览器彩排或排障：

```bash
curl --noproxy '*' -sS -X POST \
  'http://127.0.0.1:18080/api/plugins/catalog/load?package=file-leakage' \
  -H 'Content-Type: application/json' \
  --data '{"instance_id":"actrail.file-leakage","grants":{"file_policy_rules_apply":[],"env_read":[]}}'
```

Web 操作后的 REST 验证：

```bash
curl --noproxy '*' -sS \
  http://127.0.0.1:18080/api/plugins/catalog
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/plugins/runtime/config?instance_id=actrail.file-leakage'
```

`GET /api/plugins/catalog` 的 `runtime_plugins[]` 中应包含以下关键字段：

```json
{
  "instance_id": "actrail.file-leakage",
  "plugin_id": "actrail.file-leakage",
  "state": "active"
}
```

`GET /api/plugins/runtime/config?instance_id=actrail.file-leakage` 的响应中应看到：

```json
{
  "available": true,
  "instance_id": "actrail.file-leakage",
  "plugin_id": "actrail.file-leakage",
  "editable": true,
  "config": {
    "additional_allowed_roots": [],
    "candidate_max_count": 4096,
    "include_trace_working_directory": true,
    "trace_state_max_count": 256
  },
  "schema": {
    "$id": "actrail.file-leakage.config.v1",
    "type": "object"
  }
}
```

实际 `schema` 还包含每个字段的类型、约束和 `readOnly` 标记，前端根据这份完整 Schema 渲染控件。

展开 **Configuration**，确认工作目录和额外白名单可编辑，容量字段带锁并由 Schema 标记为只读。**Plugin command** 应明确显示该类 observation 插件不支持管理命令。

### 6.3 运行真实 Xiaoo 写出越界文件

终端 C 输入：

```bash
cd "$DEMO_ROOT/work"
actrailctl launch --name file-leakage-experience -- \
  xiaoo --cli run -p \
  'Use the shell to append exactly one line containing ACTRAIL_FILE_LEAK to /root/projects/AcTrail/temp/file-security-experience/outside/leaked-data.txt. Do not remove that file. Then report whether the shell command succeeded.'
```

如果仓库不在 `/root/projects/AcTrail`，把 prompt 中的绝对路径替换为终端中 `printf '%s\n' "$DEMO_ROOT"` 显示的路径。

预期终端现象：

```text
trace trace-1 entered Active
```

Xiaoo 随后报告 shell 写入成功。trace ID 不一定永远是 1，以现场输出为准。

检查文件：

```bash
sed -n '1,5p' "$DEMO_ROOT/outside/leaked-data.txt"
```

预期输出：

```text
ACTRAIL_FILE_LEAK
```

### 6.4 在 Alerts 页面展示异步出现

观察标签页 A 中保持打开的 **Stats → Alerts**。

预期现象：

- 新告警从列表顶部滑入，原有行向下移动；
- 页面 Toast 提示 `新增 1 条告警`；
- 告警标题为“存在文件泄露”；
- kind 为 `file.leakage`，severity 为 `medium`；
- 详情的“残留文件”包含 `.../temp/file-security-experience/outside/leaked-data.txt`。

Toast 不会自动消失；查看后点击右侧关闭按钮即可隐藏。

告警没有立即出现时先等待自动轮询，不要重新运行 agent。也可以点击 **Refresh**，或重复以下只读请求：

```bash
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/alerts?limit=20'
```

预期 JSON 关键内容：

```json
{
  "producer_plugin_id": "actrail.file-leakage",
  "definition_key": "file-leakage",
  "kind": "file.leakage",
  "severity": "medium",
  "payload": {
    "residual_files": [
      "/root/projects/AcTrail/temp/file-security-experience/outside/leaked-data.txt"
    ]
  }
}
```

`residual_files` 是该 trace 的全部残留越界写入候选。真实 Xiaoo 运行时还可能写入自身状态文件，例如 `~/.xiaoo/data/trace.db`，因此数组可能包含额外路径；验收条件是其中包含本节创建的 `leaked-data.txt`，而不是数组只能有一项。

用现场 trace ID 验证 trace 关联，例如 trace-1：

```bash
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/traces/1/alerts?limit=20'
```

需要准确理解：告警是 trace 结束后开始计算并异步落库，trace 结束并不构成告警写入的时间上限。

## 7. 场景二：动态文件访问治理和越界告警

### 7.1 工作原理

> 第二个插件把规则保存在自己的内存配置中。Web 配置先提交给插件，插件验证通过后把自己的规则发布给 daemon；daemon 再与其他路由合并。这里仅授权插件管理体验目录内的 deny 规则，体现最小权限。

### 7.2 在 Web 加载动态策略插件并授予最小范围

界面操作：

1. 在标签页 B 进入 **Plugins**，点击 **Refresh**。
2. 找到 `wasm.file-policy-dynamic` 候选。
3. 点击右侧 `Unloaded / Configure & load`。
4. Runtime instance name 保持 `wasm.file-policy-dynamic`。
5. 在 **Files this plugin can manage** 的 Path 填入：

```text
/root/projects/AcTrail/temp/file-security-experience/work/**
```

6. Rule types 只保留 **Deny**；取消 **Allow** 和 **Ask plugin**。
7. 点击 **Load plugin**。

预期现象：

- Loaded plugin instances 新增 `wasm.file-policy-dynamic`；
- Plugin ID 和 Instance ID 都是 `wasm.file-policy-dynamic`，但页面分别标注两者含义；
- purpose 的接口值是 `control-decider`，UI 标签显示为 `controller`；
- Host grants 包含 `file-policy.rules.read`、`file-policy.rules.match-dry-run`、`file-policy.rules.validate` 三个只读能力，以及一条：

```text
file-policy.rules.apply:kind=deny,path=/root/projects/AcTrail/temp/file-security-experience/work/**
```

REST 等价加载命令。Web 已加载成功后不要重复执行：

```bash
curl --noproxy '*' -sS -X POST \
  'http://127.0.0.1:18080/api/plugins/catalog/load?package=file-policy-dynamic' \
  -H 'Content-Type: application/json' \
  --data "{\"instance_id\":\"wasm.file-policy-dynamic\",\"grants\":{\"file_policy_rules_apply\":[{\"decision\":\"deny\",\"path_scope\":\"$DEMO_ROOT/work/**\"}]}}"
```

Web 操作后的 REST 验证：

```bash
curl --noproxy '*' -sS \
  http://127.0.0.1:18080/api/plugins/catalog
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/plugins/runtime/config?instance_id=wasm.file-policy-dynamic'
```

首次加载的配置应为：

```json
{"rules":[]}
```

### 7.3 可选安全点：证明 Host grant 不能被配置越过

这个步骤用于体验 Host grant 的安全边界；只验证基本读写治理时可以跳过。

1. 展开动态插件的 **Configuration**。
2. 在 File access routes 点击 **Add entry**。
3. Decision 从下拉选项选择 **Deny**。
4. Path 填 `/etc/passwd`，Priority 填 `20`，Rule ID 留空。
5. 点击 **Test configuration**。

预期现象：

- 页面显示配置错误；
- **Update configuration** 保持不可用；
- 错误指出插件没有对 `/etc/passwd` 的 deny apply grant；
- 插件当前内存配置仍然是 `{"rules":[]}`。

REST 校验等价命令：

```bash
curl --noproxy '*' -sS -X POST \
  'http://127.0.0.1:18080/api/plugins/runtime/config/validate?instance_id=wasm.file-policy-dynamic' \
  -H 'Content-Type: application/json' \
  --data '{"config":{"rules":[{"decision":"deny","path":"/etc/passwd","priority":20}]}}'
```

预期返回：

```json
{
  "valid": false,
  "errors": [
    "rule 0: missing file-policy.rules.apply grant for deny /etc/passwd"
  ]
}
```

删除这条无效草稿，再继续正式规则。

### 7.4 通过 Configuration 下发有效规则

界面操作：

1. 在 File access routes 点击 **Add entry**。
2. Decision 从下拉选项选择 **Deny**。
3. Path 填：

```text
/root/projects/AcTrail/temp/file-security-experience/work/private.txt
```

4. Priority 填 `20`；该字段默认值是 `10`，这里显式改为 `20` 便于识别。
5. Rule ID 留空，让插件生成稳定 ID。Rule ID 不是必填字段，`minLength` 只约束已经填写的值；留空提交后插件会分配 `dynamic-1`。
6. Gray target 留空；它只用于 **Ask plugin** 决策。
7. 点击 **Test configuration**。
8. 看到 `Test passed — ready to update` 后，点击 **Update configuration**。

测试结果绑定到测试时的完整草稿。测试成功后只要再修改任一字段，**Update configuration** 就会重新禁用，必须再次点击 **Test configuration**。

预期现象：

- 更新前必须测试成功，Update 按钮才可用；
- 更新成功提示 `Runtime configuration updated.`；
- Rule ID 自动变成 `dynamic-1`；
- Decision 是选择控件，不是自由文本输入；
- 页面保持规则字段对齐，无横向溢出。

如果不使用 Web，等价 REST 操作分两步。Web 已经更新后不要再执行 POST 更新：

```bash
curl --noproxy '*' -sS -X POST \
  'http://127.0.0.1:18080/api/plugins/runtime/config/validate?instance_id=wasm.file-policy-dynamic' \
  -H 'Content-Type: application/json' \
  --data "{\"config\":{\"rules\":[{\"decision\":\"deny\",\"path\":\"$DEMO_ROOT/work/private.txt\",\"priority\":20}]}}"

curl --noproxy '*' -sS -X POST \
  'http://127.0.0.1:18080/api/plugins/runtime/config?instance_id=wasm.file-policy-dynamic' \
  -H 'Content-Type: application/json' \
  --data "{\"config\":{\"rules\":[{\"decision\":\"deny\",\"path\":\"$DEMO_ROOT/work/private.txt\",\"priority\":20}]}}"
```

预期校验返回 `"valid":true`，更新响应的 rule 包含 `"rule_id":"dynamic-1"`。

Web 操作后的只读 REST 验证：

```bash
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/plugins/runtime/config?instance_id=wasm.file-policy-dynamic'
```

### 7.5 用 Plugin command 查询实际匹配结果

展开 **Plugin command**，每行输入一个参数：

```text
rule
dry-run
/root/projects/AcTrail/temp/file-security-experience/work/private.txt
```

点击 **Send command**。

预期 stdout：

```text
matched=true decision=deny rule_id=dynamic-1 path=/root/projects/AcTrail/temp/file-security-experience/work/private.txt revision=1
```

这个命令只查询 daemon 当前合并后的实际路由，不修改配置。命令成功后 Web 会重新读取插件配置，配置内容应保持不变。

REST 等价命令：

```bash
curl --noproxy '*' -sS -X POST \
  'http://127.0.0.1:18080/api/plugins/runtime/command?instance_id=wasm.file-policy-dynamic' \
  -H 'Content-Type: application/json' \
  --data "{\"argv\":[\"rule\",\"dry-run\",\"$DEMO_ROOT/work/private.txt\"]}"
```

再读取一次配置，证明只读命令没有修改插件内存：

```bash
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/plugins/runtime/config?instance_id=wasm.file-policy-dynamic'
```

### 7.6 运行真实 Xiaoo 验证默认放行和动态拒绝

确保标签页 A 仍停留在 **Stats → Alerts**，然后在终端 C 输入：

```bash
cd "$DEMO_ROOT/work"
actrailctl launch --name dynamic-policy-experience -- \
  xiaoo --cli run -p \
  'Use shell commands to read /root/projects/AcTrail/temp/file-security-experience/work/public.txt and then /root/projects/AcTrail/temp/file-security-experience/work/private.txt. Report the exact content or operating-system error for each file. Do not modify either file.'
```

仓库路径不同时，同样替换 prompt 中的两个绝对路径。

预期终端现象：

- 新 trace 进入 Active，例如 `trace trace-2 entered Active`；
- Xiaoo 成功读到 `public file access example`；
- Xiaoo 读取 `private.txt` 时报告 `Permission denied` 或等价的操作系统拒绝；
- agent 本身仍可完成并汇报结果。

这里的公开文件没有匹配任何规则，因此由 daemon 的 `default_decision = "allow"` 放行；私密文件命中插件发布的 `dynamic-1`，在快路径被拒绝。

### 7.7 展示 enforcement 审计证据

使用现场 trace ID，例如 trace-2：

```bash
actrailviewer events --trace-id 2 | \
  grep -E 'Enforcement|public\.txt|private\.txt|decision=(allow|deny)'
```

预期至少看到两条 enforcement 事件：

```text
open decision=allow path=.../public.txt result=allowed ... decision_source=default
open decision=deny path=.../private.txt rule_id=dynamic-1 result=denied ... decision_source=rule
```

这证明“操作系统结果”“治理审计”和“告警”是三个可以相互印证、但职责不同的结果。

### 7.8 展示新的越界访问告警

观察标签页 A 的 **Stats → Alerts**。

新告警到达后，列表不会改变当前选中的旧告警。点击列表顶部标题为 `Out-of-bound file access denied` 的新告警，再查看右侧详情。

预期现象：

- 新告警从列表顶部滑入，文件泄露告警下移；
- Toast 提示 `新增 1 条告警`；
- kind 为 `file.access.boundary-violation`；
- 标题为 `Out-of-bound file access denied`；当前 daemon 内置告警标题仍使用英文；
- severity 为 `high`；
- producer 是 `actraild.enforcement`；
- payload 中显示 `dynamic-1`、受保护路径、`wasm.file-policy-dynamic` 和 `fast-path-deny`。

如果上一条 Toast 尚未关闭，本次新告警会用新的计数提示覆盖它；查看后可手动关闭。

REST 验证：

```bash
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/alerts?limit=20'
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/traces/2/alerts?limit=20'
```

预期最新一条告警包含：

```json
{
  "producer_plugin_id": "actraild.enforcement",
  "definition_key": "file-access-boundary-violation",
  "kind": "file.access.boundary-violation",
  "severity": "high",
  "payload": {
    "decision_source": "fast-path-deny",
    "matched_rule_path": "/root/projects/AcTrail/temp/file-security-experience/work/private.txt",
    "operation": "open",
    "path": "/root/projects/AcTrail/temp/file-security-experience/work/private.txt",
    "policy_owner_instance_id": "wasm.file-policy-dynamic",
    "process_id": 3,
    "rule_id": "dynamic-1"
  }
}
```

`process_id` 是该 trace 内触发访问的 AcTrail 进程 ID，具体数字以现场响应为准，不是操作系统 PID。

## 8. 收尾和恢复环境

确认最终两条告警及其详情后，在 **Plugins** 页面分别点击两个 active 控件卸载：

1. `wasm.file-policy-dynamic` → **Unload plugin**；
2. `actrail.file-leakage` → **Unload plugin**。

预期现象：Loaded plugin instances 变为 0，两个包重新出现在 Plugin candidates。卸载 `wasm.file-policy-dynamic` 时，该实例发布的 `dynamic-1` 会立即从 daemon 的合并路由中撤销；后续 trace 不会再因这条规则拒绝 `private.txt`，但其他插件或静态规则仍可能产生独立约束。

REST 等价卸载命令；如果已经通过 Web 卸载，不要重复执行：

```bash
curl --noproxy '*' -sS -X POST \
  'http://127.0.0.1:18080/api/plugins/runtime/unload?instance_id=wasm.file-policy-dynamic'
curl --noproxy '*' -sS -X POST \
  'http://127.0.0.1:18080/api/plugins/runtime/unload?instance_id=actrail.file-leakage'
```

只读验证：

```bash
curl --noproxy '*' -sS \
  http://127.0.0.1:18080/api/plugins/catalog
```

预期 `runtime_plugin_count` 为 `0`，两个 package 的 `loaded_instances` 都为空。

在终端 B 按 `Ctrl-C` 停止 Web。终端 A 输入：

```bash
actraild stop
actrailctl clean
rm -rf /root/projects/AcTrail/temp/file-security-experience
```

预期现象：daemon 停止，默认数据库和日志被清理，体验数据目录被删除。`~/.actrail/plugins` 中的候选包会保留，供下次使用。

## 9. 故障排查顺序

必须按实际运行路径排查，不要先猜 UI 或插件内部问题。

### 9.1 Web 看不到候选插件

输入：

```bash
printf '%s\n' "$HOME"
find "$HOME/.actrail/plugins" -maxdepth 2 -type f -print | sort
curl --noproxy '*' -sS http://127.0.0.1:18080/api/plugins/catalog
```

检查 catalog 中的 `directory`、`activation_ready`、`issue` 和 `warnings`。最常见原因是安装和运行 Web 使用了不同用户的 `HOME`。

### 9.2 daemon 因静态规则文件缺失而启动失败

当前源码允许 `/etc/actrail/enforcement-rules.conf` 缺失或为空。如果日志仍显示：

```text
read enforcement rules /etc/actrail/enforcement-rules.conf: No such file or directory
```

说明 `/usr/local/bin/actraild` 是旧产物。回到 4.2，显式执行 `cargo build --workspace --release` 后重新安装。不要为绕过旧二进制而临时创建 bootstrap 规则文件。

### 9.3 动态插件加载按钮不可用

确认：

- Path 是绝对路径，目录递归范围以 `/**` 结尾；
- 每个范围至少选择一种 Rule type；
- instance name 无首尾空格；
- catalog 中 `activation_ready=true`。

### 9.4 普通文件也被拒绝

输入：

```bash
curl --noproxy '*' -sS \
  'http://127.0.0.1:18080/api/plugins/runtime/config?instance_id=wasm.file-policy-dynamic'
export TRACE_ID=2
actrailviewer events --trace-id "$TRACE_ID" | grep Enforcement
```

把示例中的 `2` 替换为 `trace trace-N entered Active` 中的数字 `N`。确认没有旧插件规则或静态规则匹配普通文件，且默认 decision 仍是 allow。

### 9.5 私密文件没有被拒绝

依次检查：

1. `actrailctl launch` 输出是否请求了 `enforcement-file-permission-fanotify`；
2. 插件是否 active；
3. Configuration 是否确实包含 `dynamic-1`；
4. `rule dry-run` 是否返回 `matched=true decision=deny`；
5. prompt 中的绝对路径是否与规则完全一致；
6. `actrailviewer events` 是否存在相应 Enforcement 事件。

直接在普通 shell 中运行 `cat` 不能验证 trace-scoped enforcement。

### 9.6 文件泄露告警没有出现

依次检查：

1. agent 的写入是否成功；
2. 写入路径是否确实在 trace 工作目录之外；
3. 目标文件在 trace 结束后是否仍存在；
4. 插件 status 的 `last_error` 是否为 `none`；
5. Alerts 页面是否仍在轮询，REST 是否已经返回告警；
6. daemon 日志是否显示 trace finalization 和 post-trace analysis 完成。

file-leakage 只看成功的 `write`/`writev`。只创建未写入、写入失败、写入允许目录，或在 trace 结束前删除候选文件，都不应产生该告警。

## 10. 当前真实测试发现的未闭环风险

以下内容不影响当前流程完成，但开始体验前应知晓：

1. **安装器可能复制旧主程序。** `install-release.sh` 只在 release 二进制缺失时构建主程序，不能单独承担“部署最新源码”的职责。本指南已通过显式 release build 规避，后续应考虑修复安装器本身。
2. **file-leakage 的 command REST 错误文案不准确。** Web 会正确显示 observation 插件不支持管理命令，但直接 POST command 当前返回“plugin instance ... not found”，即使实例实际 active。不要调用该写接口；用 catalog 中的 purpose 和 Web 的禁用说明验证能力边界。
3. **动态配置不能视为跨重启持久化状态。** 插件/daemon 重启后必须重新读取并确认规则，不要依赖上一次操作残留状态。
4. **真实 agent 输出存在外部依赖。** 模型认证、网络、配额和 agent 自身工具选择都会影响耗时；开始前必须执行 4.6 和两段完整预检。
5. **告警到达时间不能与 trace 终态绑定。** 前端轮询和手动 REST 查询只能观察最终落库结果，不能把一次没有立即返回当成“不会产生告警”。
6. **浏览器视觉需要在实际显示尺寸复查。** 本次真实后端和 Xiaoo E2E 已通过，但自动化环境没有完成真实浏览器截图检查；显示分辨率、浏览器缩放和 Arc 主题下仍需人工确认无溢出。
7. **不同内核的拒绝文案可能不同。** 当前实测 Xiaoo 报告 `Permission denied`；其他 libc、shell 或 agent 可能展示等价的 `Operation not permitted`。验收依据应是访问失败、Enforcement deny 审计和 boundary alert 三者一致，而不是绑定一条英文错误文本。

## 11. 本指南的真实验证记录

验证日期：2026-07-22；平台：Linux 5.10、aarch64；运行身份：root；配置：`/etc/actrail/actraild.conf` 默认配置加 fanotify enforcement capability 和 `enabled=true`，没有静态 enforcement 规则。

实测结果：

- release workspace、前端和两个 Wasm 插件构建成功；
- Web catalog 发现 2 个可加载候选；
- file-leakage 以自动 grants 加载成功；
- 真实 Xiaoo trace-1 成功写出工作目录外的泄露文件；
- alert-1 为 `file.leakage`，payload 包含目标残留绝对路径；Xiaoo 自身的其他越界状态文件也可能同时出现；
- 动态插件仅以体验目录 deny grant 加载成功；
- `/etc/passwd` 越权配置校验返回 `valid=false`，现有配置未改变；
- 有效 deny 配置生成 `dynamic-1`，dry-run 返回匹配；
- 真实 Xiaoo trace-2 成功读取 `public.txt`，读取 `private.txt` 得到 Permission denied；
- trace-2 审计同时记录 default allow 和 rule deny；
- alert-2 为 `file.access.boundary-violation`，producer 为 `actraild.enforcement`，decision source 为 `fast-path-deny`；
- 两个插件通过 actrailweb REST 成功卸载；
- 本次 daemon、Web、数据库、日志和临时体验目录已清理；未终止机器上不属于本次验证的其他 AcTrail 进程。
