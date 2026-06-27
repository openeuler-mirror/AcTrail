# WIT Component 灰名单文件控制插件

类别：WIT component 控制决策插件。

这个示例使用 Rust 编写 WebAssembly Component Model 插件。插件用于 fanotify 灰名单文件访问慢路径，并通过 `read-config` 读取插件自己的配置后返回允许决策。

文件：

- `plugin.toml`：插件 manifest。
- `config.toml`：插件自己的 TOML 配置。
- `component-allow.wasm`：已编译的 component artifact。
- `fixture-src/`：Rust 源码。

重新编译：

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wit-component/control-graylist/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_control_graylist_fixture.wasm ../component-allow.wasm
```
