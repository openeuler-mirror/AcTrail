"""Shared helpers for real agent trace E2E cases."""

from .actions import (
    count_action_rows,
    require_complete_llm_action,
    require_complete_llm_exchange,
    require_llm_exchange_graph,
    require_web_action_tree_projection,
    wait_for_actions,
    wait_for_llm_exchange_actions,
)
from .config import (
    DEFAULT_OPERATOR_CONFIG_PATH,
    actrail_command,
    clean_configured_paths,
    operator_config_path,
    read_config,
    render_config,
    repo_root,
    require_binary,
    require_root,
    required,
    run_checked,
)
from .otel import emit_llm_otel_evidence, export_otel, otel_attrs, otel_spans, require_otel_span
from .payloads import (
    require_complete_payload_rows,
    require_complete_payload_rows_any,
    wait_for_payloads,
    wait_for_payloads_any,
)
from .process import (
    launch_and_parse_trace,
    launch_and_parse_trace_with_daemon,
    start_daemon,
    stop_process,
)
from .process_identity import StoredTraceRoot

__all__ = [
    "DEFAULT_OPERATOR_CONFIG_PATH",
    "actrail_command",
    "clean_configured_paths",
    "count_action_rows",
    "emit_llm_otel_evidence",
    "export_otel",
    "launch_and_parse_trace",
    "launch_and_parse_trace_with_daemon",
    "operator_config_path",
    "otel_attrs",
    "otel_spans",
    "read_config",
    "render_config",
    "repo_root",
    "require_binary",
    "require_complete_llm_action",
    "require_complete_llm_exchange",
    "require_llm_exchange_graph",
    "require_web_action_tree_projection",
    "require_complete_payload_rows",
    "require_complete_payload_rows_any",
    "require_otel_span",
    "require_root",
    "required",
    "run_checked",
    "start_daemon",
    "StoredTraceRoot",
    "stop_process",
    "wait_for_actions",
    "wait_for_llm_exchange_actions",
    "wait_for_payloads",
    "wait_for_payloads_any",
]
