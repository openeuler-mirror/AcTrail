use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use tls_payload_sync::{
    ENV_BINARY, ENV_EVENT_SOCKET, ENV_PLAN_BUNDLE, ENV_POINTS, ENV_PROVIDER, PlanLookupResponse,
    RuntimePlanDescriptor, decode_runtime_plan, lookup_runtime_plan,
};

use super::codec::parse_points;
use super::state::HookPoint;

static PLAN_CACHE: OnceLock<Mutex<BTreeMap<PathBuf, Option<RuntimePlan>>>> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RuntimePlan {
    pub(super) target: PathBuf,
    pub(super) binary: PathBuf,
    pub(super) provider: String,
    pub(super) points: Vec<HookPoint>,
}

pub(super) fn current_runtime_plan() -> Result<Option<RuntimePlan>, String> {
    let current_exe = current_exe()?;
    if let Some(plan) = cached_plan(&current_exe)? {
        return Ok(plan);
    }
    let plan = resolve_current_runtime_plan(&current_exe)?;
    store_cached_plan(current_exe, plan.clone())?;
    Ok(plan)
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("missing required runtime env {name}"))
}

fn cached_plan(current_exe: &PathBuf) -> Result<Option<Option<RuntimePlan>>, String> {
    let cache = PLAN_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    cache
        .lock()
        .map_err(|_| "runtime plan cache mutex poisoned".to_string())
        .map(|cache| cache.get(current_exe).cloned())
}

fn store_cached_plan(current_exe: PathBuf, plan: Option<RuntimePlan>) -> Result<(), String> {
    let cache = PLAN_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    cache
        .lock()
        .map_err(|_| "runtime plan cache mutex poisoned".to_string())?
        .insert(current_exe, plan);
    Ok(())
}

fn resolve_current_runtime_plan(current_exe: &PathBuf) -> Result<Option<RuntimePlan>, String> {
    if let Some(value) = std::env::var_os(ENV_PLAN_BUNDLE) {
        if let Some(plan) = select_bundle_plan(&value.to_string_lossy(), current_exe)? {
            return Ok(Some(plan));
        }
    }
    if let Some(plan) = legacy_runtime_plan(current_exe)? {
        return Ok(Some(plan));
    }
    lookup_daemon_plan(current_exe)
}

fn legacy_runtime_plan(current_exe: &PathBuf) -> Result<Option<RuntimePlan>, String> {
    let Some(binary) = std::env::var_os(ENV_BINARY) else {
        return Ok(None);
    };
    let plan = RuntimePlan {
        target: PathBuf::from(binary.clone()),
        binary: PathBuf::from(binary),
        provider: required_env(ENV_PROVIDER)?,
        points: parse_points(&required_env(ENV_POINTS)?)?,
    };
    if same_binary(&plan.binary, current_exe) || mapped_probe_binary(&plan.binary) {
        Ok(Some(plan))
    } else {
        Ok(None)
    }
}

fn lookup_daemon_plan(current_exe: &PathBuf) -> Result<Option<RuntimePlan>, String> {
    let Some(socket_path) = std::env::var_os(ENV_EVENT_SOCKET) else {
        return Ok(None);
    };
    let socket_path = PathBuf::from(socket_path);
    match lookup_runtime_plan(&socket_path, current_exe) {
        Ok(PlanLookupResponse::Found(plan)) => descriptor_to_runtime_plan(plan).map(Some),
        Ok(PlanLookupResponse::Unsupported { .. }) => Ok(None),
        Err(error) => Err(format!(
            "dynamic TLS plan lookup for {} failed: {error}",
            current_exe.display()
        )),
    }
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
    let plan = decode_runtime_plan(value).map_err(|error| error.to_string())?;
    descriptor_to_runtime_plan(plan)
}

fn descriptor_to_runtime_plan(plan: RuntimePlanDescriptor) -> Result<RuntimePlan, String> {
    Ok(RuntimePlan {
        target: plan.target,
        binary: plan.binary,
        provider: plan.provider,
        points: parse_points(&plan.points)?,
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
