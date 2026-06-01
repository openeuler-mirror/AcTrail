//! Process-context enrichment for lifecycle observations.

use std::collections::BTreeMap;

pub fn lifecycle_metadata(pid: u32) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    // Live lifecycle executable comes from explicit exec observations, not racy /proc/<pid>/exe.
    insert_if_present(&mut metadata, "cwd", read_link(pid, "cwd"));
    insert_if_present(&mut metadata, "comm", read_trimmed(pid, "comm"));
    insert_cmdline(&mut metadata, pid);
    insert_status_identity(&mut metadata, pid);
    insert_stat_process_group(&mut metadata, pid);
    metadata
}

fn insert_cmdline(metadata: &mut BTreeMap<String, String>, pid: u32) {
    let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return;
    };
    let argv = raw
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .filter_map(|part| String::from_utf8(part.to_vec()).ok())
        .collect::<Vec<_>>();
    if argv.is_empty() {
        return;
    }
    metadata.insert("argv".to_string(), argv.join("\n"));
    metadata.insert("argv_count".to_string(), argv.len().to_string());
    metadata.insert("command_line".to_string(), argv.join(" "));
}

fn insert_status_identity(metadata: &mut BTreeMap<String, String>, pid: u32) {
    let Ok(raw) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return;
    };
    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("PPid:") {
            metadata.insert("ppid".to_string(), value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Uid:") {
            insert_quad(metadata, "uid", value);
        } else if let Some(value) = line.strip_prefix("Gid:") {
            insert_quad(metadata, "gid", value);
        } else if let Some(value) = line.strip_prefix("VmPeak:") {
            insert_status_number(metadata, "vm_peak_kb", value);
        } else if let Some(value) = line.strip_prefix("VmSize:") {
            insert_status_number(metadata, "vm_size_kb", value);
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            insert_status_number(metadata, "vm_hwm_kb", value);
        } else if let Some(value) = line.strip_prefix("VmRSS:") {
            insert_status_number(metadata, "vm_rss_kb", value);
        } else if let Some(value) = line.strip_prefix("RssAnon:") {
            insert_status_number(metadata, "rss_anon_kb", value);
        } else if let Some(value) = line.strip_prefix("RssFile:") {
            insert_status_number(metadata, "rss_file_kb", value);
        } else if let Some(value) = line.strip_prefix("RssShmem:") {
            insert_status_number(metadata, "rss_shmem_kb", value);
        } else if let Some(value) = line.strip_prefix("Threads:") {
            insert_status_number(metadata, "threads", value);
        }
    }
}

fn insert_status_number(metadata: &mut BTreeMap<String, String>, key: &str, raw: &str) {
    if let Some(value) = raw.split_whitespace().next() {
        metadata.insert(key.to_string(), value.to_string());
    }
}

fn insert_stat_process_group(metadata: &mut BTreeMap<String, String>, pid: u32) {
    const STAT_PPID_INDEX: usize = 1;
    const STAT_PGRP_INDEX: usize = 2;
    const STAT_SESSION_INDEX: usize = 3;

    let Ok(raw) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return;
    };
    let Some(after_comm) = raw.rsplit_once(") ") else {
        return;
    };
    let fields = after_comm.1.split_whitespace().collect::<Vec<_>>();
    for (key, index) in [
        ("stat_ppid", STAT_PPID_INDEX),
        ("process_group_id", STAT_PGRP_INDEX),
        ("session_id", STAT_SESSION_INDEX),
    ] {
        if let Some(value) = fields.get(index) {
            metadata.insert(key.to_string(), (*value).to_string());
        }
    }
}

fn insert_quad(metadata: &mut BTreeMap<String, String>, prefix: &str, raw: &str) {
    let values = raw.split_whitespace().collect::<Vec<_>>();
    for (key, value) in [
        ("real", values.first()),
        ("effective", values.get(1)),
        ("saved", values.get(2)),
        ("fs", values.get(3)),
    ] {
        if let Some(value) = value {
            metadata.insert(format!("{prefix}_{key}"), (*value).to_string());
        }
    }
}

fn read_link(pid: u32, entry: &str) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/{entry}"))
        .ok()
        .map(|value| value.display().to_string())
}

fn read_trimmed(pid: u32, entry: &str) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/{entry}"))
        .ok()
        .map(|value| value.trim_end().to_string())
        .filter(|value| !value.is_empty())
}

fn insert_if_present(
    metadata: &mut BTreeMap<String, String>,
    key: &'static str,
    value: Option<String>,
) {
    if let Some(value) = value {
        metadata.insert(key.to_string(), value);
    }
}
