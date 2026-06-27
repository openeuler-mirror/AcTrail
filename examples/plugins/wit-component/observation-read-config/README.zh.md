# WIT Component 配置读取观测插件

类别：WIT component 观测消费者。

这个示例使用 Rust 编写 WebAssembly Component Model 插件。插件导入 `read-config` hostcall，按需读取插件自己的 TOML 配置，并在配置内容符合预期后报告已处理记录。

文件：

- `plugin.toml`：插件 manifest。
- `config.toml`：插件自己的 TOML 配置。
- `component-observation-config.v1`：`schema_ref` 指向的 JSON Schema。
- `component-config.wasm`：已编译的 component artifact。
- `fixture-src/`：Rust 源码。

重新编译：

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wit-component/observation-read-config/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_observation_config_fixture.wasm ../component-config.wasm
```
