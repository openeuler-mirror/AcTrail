# File Leakage Alert Plugin

This official WIT component observes successful `write` and `writev` operations.
During execution it keeps a bounded candidate set for paths outside the trace
working directory and every configured additional allowed root.

At trace termination the component checks only the saved candidates and submits
a `file.leakage` alert asynchronously when files still exist. Alerts are append-only
rows in the independent `alerts` table; they are not semantic actions and are not
deduplicated per trace or definition. The alert payload contains only
`residual_files`.

Build the component with:

```bash
cargo build --release --target wasm32-wasip2 \
  --manifest-path examples/plugins/wit-component/file-leakage/Cargo.toml
```

`scripts/install-release.sh` builds the component and installs the complete
package at `~/.actrail/plugins/file-leakage` for the user running the installer.
Set `ACTRAIL_PLUGIN_DIR` to install into another absolute plugin root and set
the matching `plugins.discovery.directory` in the operator configuration.

Installation does not enable the plugin. A default `actraild init -f`
configuration has an empty, disabled startup-plugin list. Refresh the Plugins
workspace in `actrailweb`, review the requested host capabilities, and load the
discovered package explicitly. Unloading the runtime instance does not delete
the installed package.

The packaged manifest expects `actrail_file_leakage_plugin.wasm` beside the
manifest. Copying only the manifest is not a valid installation.

The manifest requests only `trace-file-state-read` and `alert-write`. Candidate,
queue, payload, and timeout limits are explicitly configurable through the
manifest, plugin config, or daemon operator config. See `README.zh.md` for the
complete operator workflow.
