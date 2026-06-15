use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tls_payload_sync::{
    ENV_BINARY, ENV_EVENT_SOCKET, ENV_PLAN_BUNDLE, ENV_POINTS, ENV_PROVIDER, PlanLookupResponse,
    RuntimePlanDescriptor, decode_runtime_plan, lookup_runtime_plan,
};

use crate::runtime::maps;

use super::codec::parse_points;
use super::state::HookPoint;

static PLAN_CACHE: OnceLock<Mutex<BTreeMap<PathBuf, Option<RuntimePlan>>>> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct RuntimePlan {
    pub(in crate::runtime) target: PathBuf,
    pub(in crate::runtime) binary: PathBuf,
    pub(in crate::runtime) provider: String,
    pub(in crate::runtime) points: Vec<HookPoint>,
}

impl RuntimePlan {
    pub(in crate::runtime) fn requires_inline_hooks(&self) -> bool {
        same_binary(&self.target, &self.binary)
    }
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

pub(in crate::runtime) fn runtime_plan_for_binary(
    binary: &Path,
) -> Result<Option<RuntimePlan>, String> {
    let binary = canonical(binary);
    if let Some(plan) = cached_plan(&binary)? {
        return Ok(plan);
    }
    let plan = lookup_daemon_plan(&binary)?;
    store_cached_plan(binary, plan.clone())?;
    Ok(plan)
}

pub(in crate::runtime) fn prefetch_runtime_plan_for_binary(binary: &Path) -> Result<(), String> {
    let binary = canonical(binary);
    if cached_plan(&binary)?.is_some() {
        return Ok(());
    }
    let plan = lookup_daemon_plan_without_mapping(&binary)?;
    store_cached_plan(binary, plan)
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("missing required runtime env {name}"))
}

fn cached_plan(binary: &Path) -> Result<Option<Option<RuntimePlan>>, String> {
    let cache = PLAN_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    cache
        .lock()
        .map_err(|_| "runtime plan cache mutex poisoned".to_string())
        .map(|cache| cache.get(binary).cloned())
}

fn store_cached_plan(binary: PathBuf, plan: Option<RuntimePlan>) -> Result<(), String> {
    let cache = PLAN_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    cache
        .lock()
        .map_err(|_| "runtime plan cache mutex poisoned".to_string())?
        .insert(binary, plan);
    Ok(())
}

fn resolve_current_runtime_plan(current_exe: &Path) -> Result<Option<RuntimePlan>, String> {
    if let Some(value) = std::env::var_os(ENV_PLAN_BUNDLE) {
        if let Some(plan) = select_bundle_plan(&value.to_string_lossy(), current_exe)? {
            return Ok(Some(plan));
        }
    }
    if let Some(plan) = legacy_runtime_plan(current_exe)? {
        return Ok(Some(plan));
    }
    lookup_daemon_plan_for_current_process(current_exe)
}

fn legacy_runtime_plan(current_exe: &Path) -> Result<Option<RuntimePlan>, String> {
    let Some(binary) = std::env::var_os(ENV_BINARY) else {
        return Ok(None);
    };
    let plan = RuntimePlan {
        target: PathBuf::from(binary.clone()),
        binary: PathBuf::from(binary),
        provider: required_env(ENV_PROVIDER)?,
        points: parse_points(&required_env(ENV_POINTS)?)?,
    };
    if plan_matches_current_process(&plan, current_exe) {
        Ok(Some(plan))
    } else {
        Ok(None)
    }
}

fn lookup_daemon_plan_for_current_process(
    current_exe: &Path,
) -> Result<Option<RuntimePlan>, String> {
    let Some(socket_path) = std::env::var_os(ENV_EVENT_SOCKET) else {
        return Ok(None);
    };
    let socket_path = PathBuf::from(socket_path);
    match lookup_runtime_plan(&socket_path, current_exe) {
        Ok(PlanLookupResponse::Found(plan)) => {
            let plan = descriptor_to_runtime_plan(plan)?;
            if plan_matches_current_process(&plan, current_exe) {
                Ok(Some(plan))
            } else {
                Ok(None)
            }
        }
        Ok(PlanLookupResponse::Unsupported { .. }) => Ok(None),
        Err(error) => Err(format!(
            "dynamic TLS plan lookup for {} failed: {error}",
            current_exe.display()
        )),
    }
}

fn lookup_daemon_plan(binary: &Path) -> Result<Option<RuntimePlan>, String> {
    let Some(socket_path) = std::env::var_os(ENV_EVENT_SOCKET) else {
        return Ok(None);
    };
    let socket_path = PathBuf::from(socket_path);
    match lookup_runtime_plan(&socket_path, binary) {
        Ok(PlanLookupResponse::Found(plan)) => {
            let plan = descriptor_to_runtime_plan(plan)?;
            if plan_matches_probe_binary(&plan, binary) {
                Ok(Some(plan))
            } else {
                Ok(None)
            }
        }
        Ok(PlanLookupResponse::Unsupported { .. }) => Ok(None),
        Err(error) => Err(format!(
            "dynamic TLS plan lookup for {} failed: {error}",
            binary.display()
        )),
    }
}

fn lookup_daemon_plan_without_mapping(binary: &Path) -> Result<Option<RuntimePlan>, String> {
    let Some(socket_path) = std::env::var_os(ENV_EVENT_SOCKET) else {
        return Ok(None);
    };
    let socket_path = PathBuf::from(socket_path);
    match lookup_runtime_plan(&socket_path, binary) {
        Ok(PlanLookupResponse::Found(plan)) => {
            let plan = descriptor_to_runtime_plan(plan)?;
            if same_binary(&plan.binary, binary) {
                Ok(Some(plan))
            } else {
                Ok(None)
            }
        }
        Ok(PlanLookupResponse::Unsupported { .. }) => Ok(None),
        Err(error) => Err(format!(
            "dynamic TLS plan prefetch for {} failed: {error}",
            binary.display()
        )),
    }
}

fn select_bundle_plan(value: &str, current_exe: &Path) -> Result<Option<RuntimePlan>, String> {
    for item in value.lines().filter(|item| !item.is_empty()) {
        let plan = parse_bundle_plan(item)?;
        if same_binary(&plan.target, current_exe)
            && plan_matches_current_process(&plan, current_exe)
        {
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

fn same_binary(configured: &Path, current_exe: &Path) -> bool {
    canonical(configured) == current_exe
}

fn plan_matches_current_process(plan: &RuntimePlan, current_exe: &Path) -> bool {
    if !same_binary(&plan.target, current_exe) {
        return false;
    }
    if !(same_binary(&plan.binary, current_exe) || mapped_probe_binary(&plan.binary)) {
        return false;
    }
    if plan.provider == "openssl" && !plan.requires_inline_hooks() {
        return true;
    }
    plan.points
        .iter()
        .all(|point| maps::runtime_address(&plan.binary, point.file_offset).is_ok())
}

fn plan_matches_probe_binary(plan: &RuntimePlan, binary: &Path) -> bool {
    if !same_binary(&plan.binary, binary) {
        return false;
    }
    if plan.provider == "openssl" && !plan.requires_inline_hooks() {
        return true;
    }
    plan.points
        .iter()
        .all(|point| maps::runtime_address(&plan.binary, point.file_offset).is_ok())
}

fn mapped_probe_binary(binary: &Path) -> bool {
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
        return false;
    };
    let binary = canonical(binary);
    maps.lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .map(PathBuf::from)
        .any(|path| canonical(&path) == binary)
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
