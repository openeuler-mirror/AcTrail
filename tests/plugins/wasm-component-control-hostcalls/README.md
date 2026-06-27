# WIT component ControlDecider hostcall fixture

`component-hostcalls.wasm` is generated from `fixture-src/` with:

```bash
rustup target add wasm32-wasip2
cd tests/plugins/wasm-component-control-hostcalls/fixture-src
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/actrail_component_control_hostcalls_fixture.wasm ../component-hostcalls.wasm
```

The E2E uses the checked-in component artifact so normal release tests do not need to install a WASI target or fetch guest fixture dependencies.
