use std::path::PathBuf;

use tls_payload_sync::{ENV_BINARY, ENV_PLAN_BUNDLE, ENV_POINTS, ENV_PROVIDER};

use super::codec::{decode_hex_string, parse_points};
use super::state::HookPoint;

pub(super) struct RuntimePlan {
    pub(super) target: PathBuf,
    pub(super) binary: PathBuf,
    pub(super) provider: String,
    pub(super) points: Vec<HookPoint>,
}

pub(super) fn current_runtime_plan() -> Result<Option<RuntimePlan>, String> {
    let current_exe = current_exe()?;
    if let Some(value) = std::env::var_os(ENV_PLAN_BUNDLE) {
        return select_bundle_plan(&value.to_string_lossy(), &current_exe);
    }
    let plan = RuntimePlan {
        target: PathBuf::from(required_env(ENV_BINARY)?),
        binary: PathBuf::from(required_env(ENV_BINARY)?),
        provider: required_env(ENV_PROVIDER)?,
        points: parse_points(&required_env(ENV_POINTS)?)?,
    };
    if same_binary(&plan.binary, &current_exe) || mapped_probe_binary(&plan.binary) {
        Ok(Some(plan))
    } else {
        Ok(None)
    }
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("missing required runtime env {name}"))
}

fn select_bundle_plan(value: &str, current_exe: &PathBuf) -> Result<Option<RuntimePlan>, String> {
    for item in value.lines().filter(|item| !item.is_empty()) {
        let plan = parse_bundle_plan(item)?;
        if same_binary(&plan.target, current_exe) {
            return Ok(Some(plan));
        }
    }
    Ok(None)
}

fn parse_bundle_plan(value: &str) -> Result<RuntimePlan, String> {
    let mut parts = value.split('|');
    let target = decode_hex_string(
        parts
            .next()
            .ok_or_else(|| format!("invalid runtime plan bundle item: {value}"))?,
    )?;
    let binary = decode_hex_string(
        parts
            .next()
            .ok_or_else(|| format!("invalid runtime plan bundle item: {value}"))?,
    )?;
    let provider = decode_hex_string(
        parts
            .next()
            .ok_or_else(|| format!("invalid runtime plan bundle item: {value}"))?,
    )?;
    let points = decode_hex_string(
        parts
            .next()
            .ok_or_else(|| format!("invalid runtime plan bundle item: {value}"))?,
    )?;
    if parts.next().is_some() {
        return Err(format!("invalid runtime plan bundle item: {value}"));
    }
    Ok(RuntimePlan {
        target: PathBuf::from(target),
        binary: PathBuf::from(binary),
        provider,
        points: parse_points(&points)?,
    })
}

fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map(|path| canonical(&path))
        .map_err(|error| format!("resolve current executable: {error}"))
}

fn same_binary(configured: &PathBuf, current_exe: &PathBuf) -> bool {
    canonical(configured) == *current_exe
}

fn mapped_probe_binary(binary: &PathBuf) -> bool {
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
        return false;
    };
    let binary = canonical(binary);
    maps.lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .map(PathBuf::from)
        .any(|path| canonical(&path) == binary)
}

fn canonical(path: &PathBuf) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.clone())
}
