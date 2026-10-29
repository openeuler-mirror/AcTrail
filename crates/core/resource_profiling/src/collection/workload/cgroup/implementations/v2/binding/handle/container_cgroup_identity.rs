use super::CgroupV2BindingError;
use linux_cgroup::parse_unified_process_cgroup;
use model_core::container::{ContainerRuntime, NormalizedContainerId};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerCgroupIdentity {
    pub runtime: ContainerRuntime,
    pub container_id: NormalizedContainerId,
    pub pod_uid: Option<String>,
    pub unified_process_path: PathBuf,
    pub container_boundary_path: PathBuf,
}

impl ContainerCgroupIdentity {
    pub fn parse_container_cgroup_identity(
        cgroup_file: &str,
    ) -> Result<Option<ContainerCgroupIdentity>, CgroupV2BindingError> {
        let unified = parse_unified_process_cgroup(cgroup_file)
            .map_err(|error| CgroupV2BindingError::Malformed(error.to_string()))?;
        Self::parse_unified_container_path(&unified)
    }

    pub fn parse_unified_container_path(
        path: &Path,
    ) -> Result<Option<ContainerCgroupIdentity>, CgroupV2BindingError> {
        let components = Self::canonical_absolute_components(path)?;
        Self::parse_host_layout(path, &components)
    }
    fn parse_host_layout(
        path: &Path,
        components: &[&str],
    ) -> Result<Option<ContainerCgroupIdentity>, CgroupV2BindingError> {
        let mut pod_uid = None;
        let mut pod_boundary_index = None;
        for (index, component) in components.iter().enumerate() {
            if let Some(uid) = Self::pod_uid_from_component(component) {
                pod_uid = Some(uid);
                pod_boundary_index = Some(index);
            }

            for (prefix, runtime) in [
                ("docker-", ContainerRuntime::Docker),
                ("cri-containerd-", ContainerRuntime::Containerd),
                ("crio-", ContainerRuntime::Crio),
                ("libpod-", ContainerRuntime::Podman),
            ] {
                if let Some(raw_id) = component
                    .strip_suffix(".scope")
                    .and_then(|scope| scope.strip_prefix(prefix))
                {
                    let Some(container_id) = Self::strict_id(raw_id)? else {
                        return Ok(None);
                    };
                    return Ok(Some(Self::identity_at(
                        path,
                        components,
                        index,
                        runtime,
                        container_id,
                        pod_uid,
                    )));
                }
            }

            if index > 0
                && matches!(components[index - 1], "docker" | "libpod")
                && let Some(container_id) = Self::strict_id(component)?
            {
                let runtime = if components[index - 1] == "docker" {
                    ContainerRuntime::Docker
                } else {
                    ContainerRuntime::Podman
                };
                return Ok(Some(Self::identity_at(
                    path,
                    components,
                    index,
                    runtime,
                    container_id,
                    pod_uid,
                )));
            }

            if pod_boundary_index.is_some_and(|pod_index| index == pod_index + 1)
                && let Some(container_id) = Self::strict_id(component)?
            {
                return Ok(Some(Self::identity_at(
                    path,
                    components,
                    index,
                    ContainerRuntime::K8s,
                    container_id,
                    pod_uid,
                )));
            }
        }
        Ok(None)
    }

    fn identity_at(
        process_path: &Path,
        components: &[&str],
        boundary_index: usize,
        runtime: ContainerRuntime,
        container_id: NormalizedContainerId,
        pod_uid: Option<String>,
    ) -> ContainerCgroupIdentity {
        let boundary = components[..=boundary_index]
            .iter()
            .fold(PathBuf::from("/"), |path, component| path.join(component));
        ContainerCgroupIdentity {
            runtime,
            container_id,
            pod_uid,
            unified_process_path: process_path.to_path_buf(),
            container_boundary_path: boundary,
        }
    }

    fn strict_id(raw: &str) -> Result<Option<NormalizedContainerId>, CgroupV2BindingError> {
        if raw.len() != NormalizedContainerId::HEX_LENGTH {
            return Ok(None);
        }
        NormalizedContainerId::from_lower_hex(raw)
            .map(Some)
            .map_err(CgroupV2BindingError::Malformed)
    }

    fn canonical_absolute_components(path: &Path) -> Result<Vec<&str>, CgroupV2BindingError> {
        if !path.is_absolute() {
            return Err(CgroupV2BindingError::Malformed(
                "unified cgroup path must be absolute".to_string(),
            ));
        }
        let mut output = Vec::new();
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(value) => output.push(value.to_str().ok_or_else(|| {
                    CgroupV2BindingError::Malformed("non-UTF-8 cgroup component".to_string())
                })?),
                _ => {
                    return Err(CgroupV2BindingError::Malformed(
                        "cgroup path is not canonical".to_string(),
                    ));
                }
            }
        }
        Ok(output)
    }

    fn pod_uid_from_component(component: &str) -> Option<String> {
        let raw = if let Some(raw) = component.strip_prefix("pod") {
            raw
        } else if component.starts_with("kubepods") && component.ends_with(".slice") {
            let without_suffix = component.strip_suffix(".slice")?;
            &without_suffix[without_suffix.rfind("pod")? + 3..]
        } else {
            return None;
        };
        let normalized = raw.replace('_', "-");
        Self::is_uuid(&normalized).then_some(normalized)
    }

    fn is_uuid(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() == 36
            && bytes.iter().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => *byte == b'-',
                _ => byte.is_ascii_hexdigit(),
            })
    }

    pub(super) fn component_ancestor(boundary: &Path, membership: &Path) -> bool {
        membership == boundary || membership.starts_with(boundary)
    }
}
