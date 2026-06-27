# WIT component ControlDecider graylist fixture

`component-allow.wasm` is generated from `fixture-src/` with:

```bash
rustup target add wasm32-wasip2
cd tests/plugins/wasm-component-control-graylist/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_control_graylist_fixture.wasm ../component-allow.wasm
```

The E2E uses the checked-in component artifact so normal release tests do not need to install a WASI target or fetch guest fixture dependencies.
