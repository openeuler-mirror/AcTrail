"""Defaults shared by the real-agent Web acceptance scripts and browser checks."""

from dataclasses import asdict, dataclass, field
from http import HTTPStatus
import json


@dataclass(frozen=True)
class WebTimelineConstants:
    bin_dir: str = "target/release"
    agent_path: str = "/home/yzh/.cargo/bin/xiaoo"
    playwright_path: str = "/tmp/cliproxy-endpoint-browser/node_modules/playwright/index.mjs"
    chromium_path: str = "/home/yzh/.cache/ms-playwright/chromium-1248/chrome-linux-arm64/chrome"
    host: str = "127.0.0.1"
    port: int = 18479
    cluster_port: int = 18480
    dynamic_port: int = 0
    endpoint: str = "/api/traces/{trace_id}/actions/timeline"
    command_timeout_seconds: int = 240
    agent_timeout_seconds: int = 600
    web_stop_timeout_seconds: int = 10
    web_ready_attempts: int = 100
    health_timeout_seconds: float = 1.0
    health_poll_seconds: float = 0.1
    browser_timeout_seconds: int = 600
    content_timeout_seconds: int = 180
    process_timeout_seconds: int = 60
    cluster_command_timeout_seconds: int = 120
    server_join_timeout_seconds: int = 5
    agent_max_turns: int = 16
    task_number_count: int = 8
    network_max_turns: int = 6
    long_content_max_turns: int = 1
    extra_process_traces: int = 2
    cluster_trace_ids: tuple[int, ...] = (1, 2, 3)
    cluster_initial_imports: int = 2
    long_content_repetitions: int = 3000
    long_content_chunks: int = 4
    long_content_min_bytes: int = 150 * 1024
    argv_chunk_max_bytes: int = 60 * 1024
    content_preview_bytes: int = 7
    content_block_budget_bytes: int = 1
    timeline_small_budget: int = 1
    json_indent: int = 2
    json_root_pointer: str = ""
    http_ok: int = HTTPStatus.OK
    http_bad_request: int = HTTPStatus.BAD_REQUEST
    http_not_found: int = HTTPStatus.NOT_FOUND
    http_action_kind: int = 108
    llm_http_link_roles: tuple[int, int] = (512, 514)

    viewport: dict[str, int] = field(default_factory=lambda: {"width": 1440, "height": 1000})
    browser_wait_ms: int = 30000
    navigation_settle_ms: int = 200
    timeline_page_limit: int = 100
    filtered_page_limit: int = 2
    timeline_candidate_budget: int = 2000
    timeline_max_limit: int = 1000
    visible_page_limit: int = 250
    initial_window_max_ms: int = 30001
    retained_action_budget: int = 4000
    pagination_extra_pages: int = 2
    small_budget_extra_pages: int = 3
    cluster_max_pages: int = 5
    trace_list_page_size: int = 10
    trace_api_page_size: int = 1
    trace_list_large_page_size: int = 50
    content_page_limit: int = 2
    content_start_offset: int = 0
    content_max_children: int = 5
    content_max_page_limit: int = 250
    content_max_bytes: int = 16777216
    content_boundary_bytes: tuple[int, ...] = (1, 2, 7, 31)
    cluster_content_bytes: int = 31
    long_content_page_limit: int = 50
    long_content_max_pages: int = 20
    long_content_page_bytes: int = 64 * 1024
    long_content_prefix_bytes: int = 128 * 1024
    long_content_dom_max_bytes: int = 70 * 1024
    utf8_continuation_mask: int = 0xC0
    utf8_continuation_prefix: int = 0x80
    percent_scale: int = 100
    nanoseconds_per_ms: int = 1_000_000
    minute_ms: int = 60_000
    activity_window_ms: int = 3_600_000
    activity_edited_window_ms: int = 7_200_000
    local_datetime_characters: int = 16
    overview_click_y: int = 12
    canvas_hit_offset_x: float = 1.5
    canvas_first_row_y: int = 17
    canvas_row_height: int = 28
    max_zoom_attempts: int = 12
    initial_zoom_attempt: int = 0
    center_fraction: float = 0.5
    pan_fractions: tuple[float, ...] = (0.75, 0.25, 0.9, 0.1)
    quick_jump_fraction: float = 0.05
    quick_jump_max_left: float = 0.06
    request_gate_timeout_ms: int = 1500
    quick_jump_delay_ms: int = 250
    default_tab: str = "flame_graph"
    default_action_group: str = "Messages"
    default_settle: bool = True
    require_cancellation: bool = False
    default_filters: dict[str, str] = field(default_factory=dict)
    navigation_options: dict[str, bool] = field(default_factory=dict)


CASE_CONSTANTS = WebTimelineConstants()


if __name__ == "__main__":
    print(json.dumps(asdict(CASE_CONSTANTS)))
