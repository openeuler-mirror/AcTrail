# WIT Component Payload 读取观测插件

类别：WIT component 观测消费者。

这个示例使用 Rust 编写 WebAssembly Component Model 插件。插件导入 `read-payload` hostcall，从当前观测 batch 的 payload ref 中读取一段 payload 数据。

文件：

- `plugin.toml`：插件 manifest。
- `component-payload-read.wasm`：已编译的 component artifact。
- `fixture-src/`：Rust 源码。

重新编译：

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wit-component/observation-payload-read/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_payload_read_fixture.wasm ../component-payload-read.wasm
```
