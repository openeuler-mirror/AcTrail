# actrailweb frontend

This Vue app renders the read-only AcTrail UI. Rust owns storage/API/static serving; Vue owns client-side layout, selection, lane expansion, and detail inspection. The Action Tree tab loads only the root node initially; each node requests its direct children when expanded. Commands use a separate semantic-action list API instead of forcing a full action-tree fetch.

## Build

```sh
# From the repository root:
cargo build --release
```

The web crate embeds checked-in static assets from `../src/render/dist`. Node.js, npm, and `node_modules` are only needed when changing the frontend source; the release `actrailweb` binary serves the embedded assets without them at runtime.

After frontend source changes, regenerate the checked-in assets before committing:

```sh
npm ci --prefix crates/apps/web/frontend
npm run build --prefix crates/apps/web/frontend
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
