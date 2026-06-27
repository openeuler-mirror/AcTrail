# WIT component payload-read fixture

`component-payload-read.wasm` is generated from `fixture-src/` with:

```bash
rustup target add wasm32-wasip2
cd tests/plugins/wasm-component-payload-read/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_payload_read_fixture.wasm ../component-payload-read.wasm
```

The E2E uses the checked-in component artifact so normal release tests do not need to install a WASI target or fetch guest fixture dependencies.
