# WIT Component 控制 Hostcall 插件

类别：WIT component 控制决策插件。

这个示例使用 Rust 编写 WebAssembly Component Model 插件，并在 fanotify 灰名单决策中使用以下 hostcall：

- `query-context(c, decision-summary.v1)`：读取结构化 `decision-summary`。
- `file-access.current-match-get(f, matched-rule.v1)`：读取结构化 `file-policy-view`。
- `file-policy-rules-version-get()` 和 `file-policy-rules-apply(request)`：读取策略版本，并把命中的灰名单目标 upsert 为 allow 快路径规则。
- `file-policy-rules-list(filter, cursor, limit)`：分页读回已应用规则。
- `file-policy-rules-match-dry-run(request)`：对当前目标执行 dry-run，确认会命中 allow 快路径规则。

源码通过 `actrail_plugin_abi` 引用 `c`、`f`、`decision-summary.v1` 和 `matched-rule.v1`，避免在插件里重复硬编码 ABI 字符串。

文件：

- `plugin.toml`：插件 manifest。
- `component-hostcalls.wasm`：已编译的 component artifact。
- `fixture-src/`：Rust 源码。

重新编译：

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wit-component/control-hostcalls/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_control_hostcalls_fixture.wasm ../component-hostcalls.wasm
```
