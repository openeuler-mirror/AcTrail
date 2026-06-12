# actrailweb frontend

This Vue app renders the read-only AcTrail UI. Rust owns storage/API/static serving; Vue owns client-side layout, selection, lane expansion, and detail inspection. Opening a trace loads only the trace summary and action-tree root. Timeline, event, payload, process, diagnostic, and command data are requested only when their tabs are opened. The Action Tree tab loads only the root node initially; each node requests its direct children when expanded.

## Build

```sh
# From the repository root:
npm ci --prefix crates/apps/web/frontend
cargo build --release
```

The web crate builds the Vue app into Cargo `OUT_DIR` and embeds those static
assets in the `actrailweb` binary. Node.js, npm, and `node_modules` are build
inputs only; the release binary serves the embedded assets without them at
runtime.

Packaging environments that cannot download npm dependencies during `cargo
build` should provide prebuilt assets explicitly:

```sh
ACTRAILWEB_PREBUILT_ASSETS_DIR="$PWD/crates/apps/web/frontend/dist" cargo build --release --locked
```

`ACTRAILWEB_PREBUILT_ASSETS_DIR` must be an absolute path containing
`index.html`, `assets/app.css`, and `assets/app.js`. When this variable is set,
the Cargo build script copies those files into `OUT_DIR` and does not run npm.
When it is not set, the build script runs `npm run build` and fails if npm or
installed frontend dependencies are unavailable.

For source package creation, use the repository script from a checkout with
network access to npm:

```sh
scripts/package-source.sh --output ../src-AcTrail/AcTrail-0.2.0.tar.gz
```

Then point the RPM spec build at the packaged frontend dist:

```sh
export ACTRAILWEB_PREBUILT_ASSETS_DIR="$PWD/crates/apps/web/frontend/dist"
cargo build --release --locked
```

## Dependencies

- `vue` 3.5.35: MIT.
- `@lucide/vue` 1.17.0: ISC.
- `vite` 5.4.21 and `@vitejs/plugin-vue` 5.2.4: MIT.

## Layout constants

`src/tabs/core/action-tree/config.js` contains action tree node types, lane labels,
and UI limits:

- `GRAPH_LANES`: displayed lane names.
- `TREE_NODE_TYPES`: recursive semantic tree node categories.
- `inlineAttributeCount`: maximum attributes shown inline before the full JSON block.
- `fileActivityGroupMinActions`: minimum consecutive root file actions folded into one expandable file activity node.

`src/tabs/tableConfig.js` contains table projection and render batching:

- `initialRows = 200`: table rows projected and rendered before the user asks for more.
- `rowBatchSize = 200`: additional table rows projected by each load-more action.

CSS shell dimensions are declared as custom properties in `src/styles.css`:

- `--topbar-height`: fixed desktop header height.
- `--trace-rail-width`: desktop trace rail width.
- `--detail-panel-width`: desktop detail panel width.
- `--action-lane-width`: repeated swimlane background width.
- `--action-lane-gap`: horizontal gap between a parent node and its child lane.
- `--action-node-width`: semantic action node width.
- `--action-node-min-height`: semantic action node minimum height.
- `--action-node-center-y`: connector vertical anchor inside a node.
- `--action-row-gap`: vertical gap between sibling nodes.
- `--side-panel-sticky-top`: sticky offset for the trace rail and detail panel.
- `--table-indent-step`: indentation unit for tree-like table cells.
