# 动态文件访问策略插件

这个 WIT Component 控制插件允许操作人员在 actrailweb 中维护文件访问路由。插件实例在内存中持有自己的配置，并把配置中的规则发布给 actraild；actraild 再把这些规则与静态规则、内置规则以及其他插件规则合并，形成实际生效的文件访问路由表。

配置和管理命令是两个独立入口：

```text
Web Configuration -> 插件 config submit  -> 插件内存配置 -> actraild 路由合并
Web Plugin command -> 插件 command handler -> 插件内存配置 -> actraild 路由合并
```

命令执行成功后，Web 会重新查询插件当前配置。`help`、`rule list` 和 `rule dry-run` 等命令不会修改配置，刷新后内容保持不变；`rule upsert` 和 `rule delete` 会修改同一份插件内存配置，刷新后 Configuration 面板会显示新状态。

## 前置条件

- Linux 主机支持 fanotify permission events 和 seccomp user notification。
- actraild 以 root 或具有等价 `CAP_SYS_ADMIN` 权限的身份运行。
- 已安装 release 二进制和官方插件包。
- actraild 配置启用了 `enforcement-file-permission-fanotify` capture capability。
- `[seccomp_notify]` 中 `enabled = true`。
- `[enforcement]` 中 `enabled = true`。`seccomp_syscalls` 默认允许策略使用 `mkdir` 和 `rmdir`，`seccomp_path_max_bytes` 默认是 `4096`。只有 trace 启动时合并路由表中存在对应操作的有效规则，actrailctl 才会安装该操作的 seccomp 通知过滤器；没有目录操作规则时不会增加这条通知开销。`rules_path` 指向的静态规则文件可以不存在或不包含规则；没有匹配规则时使用 `default_decision`，默认值 `allow` 会直接放行。

release 安装程序把候选插件安装到：

```text
~/.actrail/plugins/file-policy-dynamic/
├── component-file-policy-dynamic.wasm
├── config.schema.json
├── file-policy-dynamic.config.json
└── file-policy-dynamic.plugin.toml
```

安装只创建候选包，不会自动加载插件。

## 通过 actrailweb 加载插件

1. 启动 actraild 和 actrailweb。
2. 打开 actrailweb，进入 **Plugins**。
3. 点击 **Refresh**，找到 `wasm.file-policy-dynamic` 候选插件。
4. 点击 **Configure & load**。
5. 填写易于识别的实例 ID，例如 `wasm.file-policy-dynamic`。
6. 在 **Writable file-policy scopes** 中添加插件可以发布规则的绝对路径范围。
7. 为该范围选择需要授权的 `allow`、`deny` 和 `gray` 决策类型。
8. 点击加载按钮。

路径范围支持精确路径和递归范围。例如：

```text
/srv/agent/workspace/**
```

这个授权允许插件为 `/srv/agent/workspace/` 下的文件发布规则。配置面板中的规则仍会逐条接受校验；插件不能用配置或命令越过加载时授予的范围和决策类型。

加载成功后，**Loaded plugin instances** 中会分别显示：

- **Instance ID**：本次加载的实例标识，例如 `wasm.file-policy-dynamic`。
- **Plugin ID**：插件类型标识，同一插件类型的不同实例共享此值。
- **Host grants**：本实例获得的只读能力列表和可写策略范围。

## 通过 Configuration 管理规则

展开已加载实例，再展开 **Configuration**。配置来自插件当前内存，而不是 actraild 路由表的反向映射。

每条规则包含：

| 字段 | 是否必填 | 说明 |
| --- | --- | --- |
| `rule_id` | 新规则可省略 | 插件拥有的稳定规则 ID。提交新规则时由插件生成，例如 `dynamic-1`。 |
| `decision` | 是 | `allow`、`deny` 或 `gray`。 |
| `operation` | 否 | `any`、`open`、`mkdir` 或 `rmdir`。默认 `any`，同时匹配文件打开、目录创建和目录删除。 |
| `path` | 是 | 要匹配的绝对文件或目录路径。必须位于加载时授予的 scope 内。 |
| `priority` | 是 | 路由优先级，数值越大优先级越高。默认值为 `10`。 |
| `gray_target` | gray 必填 | 接收同步判定的控制插件实例索引；非 gray 规则不能填写。 |

提交过程分为两步：

1. 修改字段后点击 **Test configuration**。actrailweb 后端、actraild 的 JSON Schema 校验以及插件语义校验都会执行。
2. 校验成功后 **Update configuration** 才会启用。点击后，配置提交给插件；插件验证并发布路由成功后才更新内存配置。

如果规则超出 Host grants、gray 规则缺少 `gray_target`、规则 ID 重复或路径无效，测试或更新会返回明确错误，插件内存配置保持原值。

单次配置文档的大小上限由 manifest 中的 `hostcall_limits.plugin_config.read_max_bytes` 控制；官方包默认允许 `65536` 字节。超过上限时，查询、校验和提交都会直接失败。

## 通过 Plugin command 管理同一份配置

展开 **Plugin command**，每行输入一个参数。例如新增 deny 规则时输入：

```text
rule
upsert
deny
/srv/agent/workspace/private.txt
--operation
open
--priority
20
```

发送成功后，Web 自动重新读取 Configuration。新规则会带有插件生成的稳定 ID。

列出插件内存中的规则：

```text
rule
list
```

输出示例：

```text
dynamic-1 deny open /srv/agent/workspace/private.txt priority=20
```

删除规则：

```text
rule
delete
dynamic-1
```

查看帮助：

```text
help
```

查看某个路径在 actraild 当前合并路由表中的匹配结果：

```text
rule
dry-run
/srv/agent/workspace/private.txt
--operation
open
```

`rule dry-run` 只查询实际生效路由，不修改插件配置。

## 命令参考

| 命令 | 是否修改配置 | 作用 |
| --- | --- | --- |
| `help` | 否 | 显示插件支持的命令。 |
| `rule list` | 否 | 列出插件当前内存配置中的规则。 |
| `rule dry-run <path> [--operation open\|mkdir\|rmdir]` | 否 | 查询 actraild 合并后的路由匹配结果；省略 operation 时查询 `open`。 |
| `rule upsert allow <path> [--operation any\|open\|mkdir\|rmdir] [--priority N]` | 是 | 新增 allow 规则；省略 operation 时使用 `any`。 |
| `rule upsert deny <path> [--operation any\|open\|mkdir\|rmdir] [--priority N]` | 是 | 新增 deny 规则；省略 operation 时使用 `any`。 |
| `rule upsert gray <path> [--operation any\|open\|mkdir\|rmdir] [--priority N] --gray-target INDEX` | 是 | 新增 gray 规则；省略 operation 时使用 `any`。 |
| `rule delete <rule-id>` | 是 | 删除插件配置中的指定规则。 |

## 验证实际访问结果

策略只作用于启用了 fanotify enforcement capability 的 AcTrail trace。普通的未跟踪进程不受 trace-scoped 策略影响。

目录操作的 seccomp filter 在 trace 启动时确定。请先加载插件并提交 `mkdir`/`rmdir`/`any` 规则，再启动 agent；运行中的 trace 无法追加 seccomp filter，因此之后新增的目录操作规则只对后续启动的 trace 生效。删除规则后，已经安装的 filter 会继续存在到该 trace 结束，但 daemon 会直接放行没有匹配规则的通知。

下面假设配置中已有：

- `/srv/agent/workspace/public.txt` 的 allow 规则；
- `/srv/agent/workspace/private.txt` 的 deny 规则。

用真实 agent 通过 AcTrail 启动，并要求它读取两个文件：

```bash
sudo target/release/actrailctl launch --config /etc/actrail/actraild.conf \
  --name dynamic-file-policy-check -- \
  xiaoo --cli run -p \
  'Read /srv/agent/workspace/public.txt and /srv/agent/workspace/private.txt. Report the result for each path and do not modify either file.'
```

预期结果：allow 文件可读取，deny 文件返回 `Permission denied`。

查看该 trace 的审计事件：

```bash
sudo target/release/actrailviewer events \
  --config /etc/actrail/actraild.conf \
  --trace-id 1
```

输出中应包含类似记录：

```text
Enforcement open decision=allow path=/srv/agent/workspace/public.txt rule_id=dynamic-2 result=allowed backend=fanotify
Enforcement open decision=deny path=/srv/agent/workspace/private.txt rule_id=dynamic-1 result=denied backend=fanotify
```

deny 和 gray-plugin deny 还会异步生成 `file.access.boundary-violation` 告警。打开 **Stats → Alerts** 可以查看按时间倒序排列的告警；告警写入不参与文件访问判定。

要验证目录创建阻断，把一条 deny 规则的 `operation` 设为 `mkdir`，路径设为 `/srv/agent/workspace/new-directory`，然后运行真实 agent：

```bash
sudo target/release/actrailctl launch -- \
  opencode run '请确认 /srv/agent/workspace/new-directory 是否可以创建。'
```

预期结果：agent 收到 `Permission denied`，目标目录不存在；该 trace 的 enforcement 审计记录 `mkdir decision=deny`，**Stats → Alerts** 出现对应的 `file.access.boundary-violation`。如果同时加载 file-leakage 插件，它会在 trace 结束后的独立异步计算中写入自己的告警，两类告警互不替代。

## 无 Web 环境下加载

CLI 加载时必须同时提供 JSON 配置文件、自动能力授权和参数化写入范围：

```bash
export INSTANCE=wasm.file-policy-dynamic
export TARGET_SCOPE=/srv/agent/workspace/**

sudo target/release/actraild --config /etc/actrail/actraild.conf plugin load \
  --manifest examples/plugins/wit-component/file-policy-dynamic/plugin.toml \
  --plugin-config examples/plugins/wit-component/file-policy-dynamic/file-policy-dynamic.config.json \
  --grant file-policy.rules.read \
  --grant file-policy.rules.match-dry-run \
  --grant file-policy.rules.validate \
  --grant "file-policy.rules.apply:kind=allow,path=$TARGET_SCOPE" \
  --grant "file-policy.rules.apply:kind=deny,path=$TARGET_SCOPE" \
  --grant "file-policy.rules.apply:kind=gray,path=$TARGET_SCOPE" \
  --instance "$INSTANCE"
```

CLI 命令与 Web 的 Plugin command 使用同一个插件 command handler：

```bash
sudo target/release/actraild --config /etc/actrail/actraild.conf plugin cmd \
  --instance "$INSTANCE" -- help
```

## 卸载插件

在 actrailweb 的 **Loaded plugin instances** 中点击实例右侧的 **Unload plugin**。卸载实例会移除该插件实例发布到 actraild 的路由；其他插件、静态配置和内置路由不受影响。

无 Web 环境时可以执行：

```bash
sudo target/release/actraild --config /etc/actrail/actraild.conf plugin unload \
  --instance wasm.file-policy-dynamic
```

## 从源码构建插件

只有修改 `fixture-src/` 下的插件源码或 WIT contract 时才需要重新构建：

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wit-component/file-policy-dynamic/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_file_policy_dynamic.wasm \
  ../component-file-policy-dynamic.wasm
```

## 文件说明

| 文件 | 说明 |
| --- | --- |
| `plugin.toml` | 插件 manifest，声明 runtime-managed JSON 配置和 Host capabilities。 |
| `file-policy-dynamic.config.json` | 默认插件配置，初始规则列表为空。 |
| `config.schema.json` | Web 与 actraild 使用的 JSON Schema。 |
| `component-file-policy-dynamic.wasm` | 可加载的 WIT Component 插件产物。 |
| `fixture-src/` | 插件 Rust 源码。 |
| `README.zh.md` | 面向产品用户和运维人员的操作说明。 |
