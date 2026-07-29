# File I/O 终态导出目标文件路径

下列为目标设计的候选影响路径；列出路径不代表每个文件都必须修改。

## File projection

- `crates/core/semantic_action_runtime/src/live/file/projection/access.rs`
- `crates/core/semantic_action_runtime/src/live/file/projection/io_action.rs`
- `crates/core/semantic_action_runtime/src/live/file/projection/summary.rs`
- `crates/core/semantic_action_runtime/src/live/file/projection/bulk_read.rs`
- `crates/core/semantic_action_runtime/src/live/file/projection/enumerate.rs`
- `crates/core/semantic_action_runtime/src/live/actions.rs`
- `crates/core/semantic_action_runtime/src/live/runtime.rs`

## Recording 与 export

- `crates/recording/runtime/src/semantic/export.rs`
- `crates/apps/daemon/src/services/live/shutdown.rs`

## 验证

- `tests/v2/regression/plugins/otel-jsonl/`
