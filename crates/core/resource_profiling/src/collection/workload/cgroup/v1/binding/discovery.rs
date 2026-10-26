use crate::ProfilingError;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub(super) struct V1Mount {
    pub(super) point: PathBuf,
    pub(super) root: PathBuf,
}
pub(crate) struct V1Discovery {
    mounts: BTreeMap<String, V1Mount>,
}
impl V1Discovery {
    pub(crate) fn discover() -> Result<Self, ProfilingError> {
        let text = std::fs::read_to_string("/proc/self/mountinfo").map_err(Self::io)?;
        let mut mounts = BTreeMap::new();
        for line in text.lines() {
            let Some((left, right)) = line.split_once(" - ") else {
                return Err(Self::invalid("malformed mountinfo"));
            };
            let fields: Vec<_> = right.split_whitespace().collect();
            if fields.first() != Some(&"cgroup") {
                continue;
            }
            let paths: Vec<_> = left.split_whitespace().collect();
            if paths.len() < 6 || fields.len() < 3 {
                return Err(Self::invalid("malformed cgroup mountinfo"));
            }
            let point = Self::unescape(paths[4])?;
            let root = Self::unescape(paths[3])?;
            for controller in fields[2]
                .split(',')
                .filter(|c| matches!(*c, "cpuacct" | "memory"))
            {
                // A less restricted mount can resolve more observer memberships.
                let mount = V1Mount {
                    point: point.clone(),
                    root: root.clone(),
                };
                if mounts.get(controller).is_none_or(|old: &V1Mount| {
                    mount.root.components().count() < old.root.components().count()
                }) {
                    mounts.insert(controller.into(), mount);
                }
            }
        }
        Ok(Self { mounts })
    }
    pub(crate) fn available(&self) -> bool {
        self.mounts.contains_key("cpuacct") && self.mounts.contains_key("memory")
    }
    pub(super) fn mount(&self, controller: &str) -> Result<&V1Mount, ProfilingError> {
        self.mounts
            .get(controller)
            .ok_or_else(|| Self::unavailable(format!("v1 {controller} hierarchy unavailable")))
    }
    pub(super) fn membership(pid: u32) -> Result<BTreeMap<String, PathBuf>, ProfilingError> {
        let text = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).map_err(Self::io)?;
        let mut result = BTreeMap::new();
        for line in text.lines() {
            let mut fields = line.splitn(3, ':');
            let _ = fields.next();
            let controllers = fields
                .next()
                .ok_or_else(|| Self::invalid("malformed cgroup membership"))?;
            let path = fields
                .next()
                .ok_or_else(|| Self::invalid("malformed cgroup membership"))?;
            for controller in controllers
                .split(',')
                .filter(|c| matches!(*c, "cpuacct" | "memory"))
            {
                result.insert(controller.into(), PathBuf::from(path));
            }
        }
        Ok(result)
    }
    fn unescape(raw: &str) -> Result<PathBuf, ProfilingError> {
        use std::os::unix::ffi::OsStringExt;
        let mut result = Vec::new();
        let bytes = raw.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                let oct = bytes
                    .get(i + 1..i + 4)
                    .ok_or_else(|| Self::invalid("invalid mount escape"))?;
                if !oct.iter().all(|b| (b'0'..=b'7').contains(b)) {
                    return Err(Self::invalid("invalid mount escape"));
                }
                let n = u16::from(oct[0] - b'0') * 64
                    + u16::from(oct[1] - b'0') * 8
                    + u16::from(oct[2] - b'0');
                result.push(u8::try_from(n).map_err(|_| Self::invalid("invalid mount escape"))?);
                i += 4;
            } else {
                result.push(bytes[i]);
                i += 1;
            }
        }
        let path = PathBuf::from(std::ffi::OsString::from_vec(result));
        if !path.is_absolute() {
            return Err(Self::invalid("mount path must be absolute"));
        }
        Ok(path)
    }
    pub(super) fn relative(path: &Path, root: &Path) -> Result<PathBuf, ProfilingError> {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| Self::unavailable("membership is outside visible mount"))?;
        if relative
            .components()
            .any(|p| !matches!(p, std::path::Component::Normal(_)))
        {
            return Err(Self::invalid("unsafe cgroup membership path"));
        }
        Ok(relative.into())
    }
    pub(crate) fn unavailable(message: impl Into<String>) -> ProfilingError {
        ProfilingError::new("cgroup_v1_unavailable", message).availability(true)
    }
    pub(super) fn invalid(message: impl Into<String>) -> ProfilingError {
        ProfilingError::new("cgroup_v1_binding", message)
    }
    pub(super) fn io(error: std::io::Error) -> ProfilingError {
        let unavailable = matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
        );
        ProfilingError::new("cgroup_v1_io", error.to_string()).availability(unavailable)
    }
}
