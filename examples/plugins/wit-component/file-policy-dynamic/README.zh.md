# 动态文件策略插件示例

这个示例演示一个 WIT Component 控制插件如何在运行中管理文件访问规则：

- 通过 `actraild plugin cmd` 给插件发送管理命令；
- 插件把 allow、deny、gray 规则提交给 AcTrail；
- 被 AcTrail 跟踪的进程访问这些文件时，可以看到放行、拒绝或进入 gray 决策的结果。

下面的流程假设你在仓库根目录执行命令。

## 前置条件

需要先有 release 二进制：

```bash
cargo build --release
```

fanotify 文件访问控制需要 root 或等价的 `CAP_SYS_ADMIN` 权限。下面命令统一用 `sudo` 演示。

这个示例目录已经包含编译好的插件产物：

```text
examples/plugins/wit-component/file-policy-dynamic/component-file-policy-dynamic.wasm
```

首次试用不需要重新编译 `.wasm`。

## 1. 准备测试目录

```bash
export DEMO_DIR=/tmp/actrail-file-policy-demo
export CONFIG=$DEMO_DIR/operator.conf
export RULES=$DEMO_DIR/rules.conf
export TARGETS=$DEMO_DIR/targets
export INSTANCE=wasm.file-policy-dynamic

rm -rf "$DEMO_DIR"
mkdir -p "$TARGETS"

printf 'allow\n' > "$TARGETS/allow.txt"
printf 'deny\n' > "$TARGETS/deny.txt"
printf 'gray allow\n' > "$TARGETS/gray-allow-1.txt"
printf 'gray deny\n' > "$TARGETS/gray-deny-1.txt"
printf 'bootstrap\n' > "$TARGETS/.bootstrap"
printf 'bootstrap allow open %s/.bootstrap\n' "$TARGETS" > "$RULES"
```

现象：`/tmp/actrail-file-policy-demo/targets/` 下有四个用于验证的文本文件，`rules.conf` 里有一条 bootstrap allow 规则。`operator.conf`、socket、SQLite、log 和 rules 不放在 `targets/` 里，避免 daemon 自己读写运行文件时触发本示例的 fanotify permission path。

## 2. 生成 demo 配置

使用默认配置模板，再应用本示例的 patch：

```bash
sudo target/release/actraild --config "$CONFIG" init \
  --patch examples/plugins/wit-component/file-policy-dynamic/operator.patch.toml \
  --force
```

预期输出：

```text
initialized config /tmp/actrail-file-policy-demo/operator.conf
```

现象：`operator.conf` 和 `rules.conf` 都在 demo 目录里。patch 只改本示例需要的路径、capability 和 enforcement 开关；默认配置里的其他字段仍由当前版本的 AcTrail 模板提供。`bootstrap` 规则只用于让 enforcement 服务有一个启动时规则，后续 allow、deny、gray 规则都由插件动态写入到 `targets/` 下。

## 3. 启动 daemon

```bash
sudo target/release/actraild --config "$CONFIG" start
```

预期现象：命令返回成功，daemon 在后台运行。

检查 daemon 状态：

```bash
sudo target/release/actraild --config "$CONFIG" status
```

预期输出显示 daemon 正在运行。

再检查 control socket：

```bash
sudo target/release/actrailctl --config "$CONFIG" doctor
```

现象：`doctor` 成功返回；如果 control socket 未就绪，会报连接错误。

## 4. 加载插件

```bash
sudo target/release/actraild --config "$CONFIG" plugin load \
  --manifest examples/plugins/wit-component/file-policy-dynamic/plugin.toml \
  --grant file-policy.rules.read \
  --grant file-policy.rules.match-dry-run \
  --grant "file-policy.rules.apply:kind=allow,path=$TARGETS/**" \
  --grant "file-policy.rules.apply:kind=deny,path=$TARGETS/**" \
  --grant "file-policy.rules.apply:kind=gray,path=$TARGETS/**" \
  --instance "$INSTANCE"
```

预期输出：

```text
loaded instance=wasm.file-policy-dynamic
warnings=none
```

确认插件已加载：

```bash
sudo target/release/actraild --config "$CONFIG" plugin list
```

预期能看到一行 `wasm.file-policy-dynamic`，状态为 `active`。

## 5. 下发 allow 规则并验证

把 `allow.txt` 设置为 allow：

```bash
sudo target/release/actraild --config "$CONFIG" plugin cmd \
  --instance "$INSTANCE" -- \
  rule upsert allow "$TARGETS/allow.txt" --priority 10
```

预期输出类似：

```text
accepted revision=1 applied=1 path=/tmp/actrail-file-policy-demo/targets/allow.txt
```

查看 dry-run 命中结果：

```bash
sudo target/release/actraild --config "$CONFIG" plugin cmd \
  --instance "$INSTANCE" -- \
  rule dry-run "$TARGETS/allow.txt"
```

预期输出包含：

```text
matched=true decision=allow
```

用一个被 AcTrail 跟踪的进程访问文件：

```bash
sudo target/release/actrailctl --config "$CONFIG" launch -- \
  python3 -c "open('$TARGETS/allow.txt', 'r', encoding='utf-8').read(); print('allow_read=ok')"
```

预期输出包含：

```text
allow_read=ok
```

## 6. 下发 deny 规则并验证

```bash
sudo target/release/actraild --config "$CONFIG" plugin cmd \
  --instance "$INSTANCE" -- \
  rule upsert deny "$TARGETS/deny.txt" --priority 10
```

预期输出类似：

```text
accepted revision=2 applied=1 path=/tmp/actrail-file-policy-demo/targets/deny.txt
```

访问被拒绝的文件：

```bash
sudo target/release/actrailctl --config "$CONFIG" launch -- \
  python3 -c "open('$TARGETS/deny.txt', 'r', encoding='utf-8').read(); print('deny_read=unexpected_ok')"
```

预期现象：命令返回非 0，Python 报 `PermissionError` 或 `Permission denied`。这说明 fanotify enforcement 拒绝了本次 open。

## 7. 下发 gray 规则并验证

这个 demo 假设当前 daemon 只加载了这一个控制插件。控制插件实例索引从 `1` 开始，因此 gray 规则使用 `--gray-target 1`。

```bash
sudo target/release/actraild --config "$CONFIG" plugin cmd \
  --instance "$INSTANCE" -- \
  rule upsert gray "$TARGETS/gray-allow-1.txt" --priority 10 --gray-target 1

sudo target/release/actraild --config "$CONFIG" plugin cmd \
  --instance "$INSTANCE" -- \
  rule upsert gray "$TARGETS/gray-deny-1.txt" --priority 10 --gray-target 1
```

预期输出类似：

```text
accepted revision=3 applied=1 path=/tmp/actrail-file-policy-demo/targets/gray-allow-1.txt
accepted revision=4 applied=1 path=/tmp/actrail-file-policy-demo/targets/gray-deny-1.txt
```

访问 hash 为偶数的 gray 文件：

```bash
sudo target/release/actrailctl --config "$CONFIG" launch -- \
  python3 -c "open('$TARGETS/gray-allow-1.txt', 'r', encoding='utf-8').read(); print('gray_allow_read=ok')"
```

预期输出包含：

```text
gray_allow_read=ok
```

访问 hash 为奇数的 gray 文件：

```bash
sudo target/release/actrailctl --config "$CONFIG" launch -- \
  python3 -c "open('$TARGETS/gray-deny-1.txt', 'r', encoding='utf-8').read(); print('gray_deny_read=unexpected_ok')"
```

预期现象：命令返回非 0，Python 报 `PermissionError` 或 `Permission denied`。

这个插件用路径 hash 的奇偶性模拟“外部动态分析服务”的结论：偶数 allow，奇数 deny。
上面的两个文件名只在 `DEMO_DIR=/tmp/actrail-file-policy-demo` 时保证这个结果；如果修改了 demo 目录，hash 奇偶性也可能变化。

## 8. 查看和删除规则

列出当前规则：

```bash
sudo target/release/actraild --config "$CONFIG" plugin cmd \
  --instance "$INSTANCE" -- \
  rule list
```

输出形状类似：

```text
revision=4
bootstrap allow /tmp/actrail-file-policy-demo/targets/.bootstrap priority=0
fp-1 allow /tmp/actrail-file-policy-demo/targets/allow.txt priority=10
fp-2 deny /tmp/actrail-file-policy-demo/targets/deny.txt priority=10
fp-3 gray /tmp/actrail-file-policy-demo/targets/gray-allow-1.txt priority=10
fp-4 gray /tmp/actrail-file-policy-demo/targets/gray-deny-1.txt priority=10
```

删除规则时使用 `rule list` 输出里的真实 rule id。比如删除 deny 规则：

```bash
sudo target/release/actraild --config "$CONFIG" plugin cmd \
  --instance "$INSTANCE" -- \
  rule delete fp-2
```

预期输出类似：

```text
accepted revision=5 deleted=fp-2
```

再次 dry-run：

```bash
sudo target/release/actraild --config "$CONFIG" plugin cmd \
  --instance "$INSTANCE" -- \
  rule dry-run "$TARGETS/deny.txt"
```

预期输出包含：

```text
matched=false decision=allow
```

因为本 demo 的 `default_decision = "allow"`，删除 deny 规则后，访问 `deny.txt` 会恢复放行。

## 9. 清理

卸载插件：

```bash
sudo target/release/actraild --config "$CONFIG" plugin unload --instance "$INSTANCE"
```

预期输出：

```text
unloaded instance=wasm.file-policy-dynamic
```

停止 daemon：

```bash
sudo target/release/actraild --config "$CONFIG" stop
```

删除 demo 目录：

```bash
sudo rm -rf "$DEMO_DIR"
```

## 命令参考

插件支持的管理命令：

| 命令 | 作用 |
| --- | --- |
| `rule upsert allow <path> [--priority N]` | 新增或更新 allow 规则。 |
| `rule upsert deny <path> [--priority N]` | 新增或更新 deny 规则。 |
| `rule upsert gray <path> [--priority N] --gray-target INDEX` | 新增或更新 gray 规则，命中后交给指定控制插件实例决策。 |
| `rule list` | 列出插件写入 AcTrail 的文件策略规则。 |
| `rule dry-run <path>` | 查看指定路径当前会命中哪条规则，不触发真实文件访问。 |
| `rule delete <rule-id>` | 删除指定规则；`rule-id` 来自 `rule list` 输出。 |

`actraild plugin cmd --instance "$INSTANCE" -- ...` 中的 `--` 是分隔符：前面是 AcTrail 的参数，后面是原样传给插件的命令参数。

## 从源码重新构建插件

只有修改了 `fixture-src/` 里的 Rust 源码时才需要重新构建：

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wit-component/file-policy-dynamic/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_file_policy_dynamic.wasm ../component-file-policy-dynamic.wasm
```

## 自动端到端验证

人工流程跑通后，可以用 E2E 脚本做回归验证：

```bash
sudo env ACTRAIL_BIN_DIR=target/release python3 tests/plugins/file-policy-dynamic/run_e2e.py
```

该脚本会启动真实 daemon，加载真实插件，验证 allow、deny、规则列表、规则删除、dry-run 和 gray hash 决策。

## 文件说明

| 文件 | 说明 |
| --- | --- |
| `plugin.toml` | 插件 manifest。 |
| `component-file-policy-dynamic.wasm` | 可直接加载的 WIT Component 插件产物。 |
| `fixture-src/` | 插件 Rust 源码。 |
| `README.zh.md` | 本说明文档。 |
