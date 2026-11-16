// Tests for the corresponding production module.
use super::parse_tracepoint_field_offset;

const LEGACY_FORK_FORMAT: &str = r#"
name: sched_process_fork
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:char parent_comm[16];	offset:8;	size:16;	signed:1;
	field:pid_t parent_pid;	offset:24;	size:4;	signed:1;
	field:char child_comm[16];	offset:28;	size:16;	signed:1;
	field:pid_t child_pid;	offset:44;	size:4;	signed:1;
"#;

const DYNAMIC_FORK_FORMAT: &str = r#"
name: sched_process_fork
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:__data_loc char[] parent_comm;	offset:8;	size:4;	signed:0;
	field:pid_t parent_pid;	offset:12;	size:4;	signed:1;
	field:__data_loc char[] child_comm;	offset:16;	size:4;	signed:0;
	field:pid_t child_pid;	offset:20;	size:4;	signed:1;
"#;

#[test]
fn parses_legacy_fork_child_pid_offset() {
    assert_eq!(
        parse_tracepoint_field_offset(LEGACY_FORK_FORMAT, "child_pid"),
        Some(44)
    );
}

#[test]
fn parses_dynamic_fork_child_pid_offset() {
    assert_eq!(
        parse_tracepoint_field_offset(DYNAMIC_FORK_FORMAT, "child_pid"),
        Some(20)
    );
}

#[test]
fn rejects_missing_zero_or_non_u32_tracepoint_fields() {
    assert_eq!(
        parse_tracepoint_field_offset(DYNAMIC_FORK_FORMAT, "missing_pid"),
        None
    );
    assert_eq!(
        parse_tracepoint_field_offset(
            "field:pid_t child_pid; offset:0; size:4; signed:1;",
            "child_pid"
        ),
        None
    );
    assert_eq!(
        parse_tracepoint_field_offset(
            "field:long child_pid; offset:20; size:8; signed:1;",
            "child_pid"
        ),
        None
    );
}
