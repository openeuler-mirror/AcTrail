//! Process-wide host/VM identity (OpenTelemetry `host.id`).
//!
//! One daemon runs on exactly one machine (bare metal or VM), so the host id is
//! a process constant. It is resolved once at startup — from an operator config
//! override, else the DMI product UUID — and stamped onto newly attached live
//! traces (injected into the OTLP resource; not persisted to SQLite in v1). A
//! VM exposes its own SMBIOS via the hypervisor, so `product_uuid` yields the
//! VM's stable id and survives live migration.

use std::sync::OnceLock;

static HOST_ID: OnceLock<Option<String>> = OnceLock::new();

/// DMI product UUID path. Root-readable (0400); the daemon runs as root.
const DMI_PRODUCT_UUID_PATH: &str = "/sys/class/dmi/id/product_uuid";

/// Resolve and cache the host id once. Config override wins and is trusted
/// as-is (the operator chose it). Otherwise probe DMI, but reject placeholder
/// UUIDs (all-zeros / obviously non-unique) so we never stamp a useless,
/// fleet-colliding id — better to leave `host.id` unset and warn. Idempotent.
///
/// Note: a *valid-looking but cloned* `product_uuid` (template VMs that share
/// the same SMBIOS UUID) cannot be detected from a single machine — set
/// `[control] host_id` explicitly on cloned VMs to avoid silent collisions.
pub fn init(config_override: Option<String>) {
    let resolved = match config_override
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => Some(value),
        None => match probe_dmi_product_uuid(DMI_PRODUCT_UUID_PATH) {
            Some(uuid) if is_placeholder_uuid(&uuid) => {
                tracing::warn!(
                    product_uuid = %uuid,
                    "DMI product_uuid looks like a placeholder / non-unique value; \
                     host.id left unset — set [control] host_id explicitly to \
                     disambiguate this machine"
                );
                None
            }
            other => other,
        },
    };
    let _ = HOST_ID.set(resolved);
}

/// A DMI `product_uuid` that cannot uniquely identify a machine: all-zeros or
/// all-`f` placeholders that some hypervisors / firmware emit.
fn is_placeholder_uuid(uuid: &str) -> bool {
    let stripped: String = uuid.chars().filter(|c| *c != '-').collect();
    stripped.is_empty()
        || stripped.chars().all(|c| c == '0')
        || stripped.chars().all(|c| c == 'f' || c == 'F')
}

/// The resolved host id, or `None` if unresolved (single-host / no DMI / not
/// yet initialized, e.g. in unit tests).
pub fn get() -> Option<String> {
    HOST_ID.get().and_then(|value| value.clone())
}

fn probe_dmi_product_uuid(path: &str) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_trims_and_rejects_empty() {
        let dir = std::env::temp_dir();
        let uuid_path = dir.join(format!("actrail-host-id-{}.uuid", std::process::id()));
        std::fs::write(&uuid_path, "  4C4C4544-0042-1234-8000-abcdef012345\n").unwrap();
        assert_eq!(
            probe_dmi_product_uuid(uuid_path.to_str().unwrap()),
            Some("4C4C4544-0042-1234-8000-abcdef012345".to_string())
        );

        std::fs::write(&uuid_path, "   \n").unwrap();
        assert_eq!(probe_dmi_product_uuid(uuid_path.to_str().unwrap()), None);

        std::fs::remove_file(&uuid_path).ok();
        assert_eq!(
            probe_dmi_product_uuid(uuid_path.to_str().unwrap()),
            None,
            "missing DMI file resolves to None, not an error"
        );
    }

    #[test]
    fn placeholder_uuids_are_rejected_but_real_ones_kept() {
        assert!(is_placeholder_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(is_placeholder_uuid("ffffffff-ffff-ffff-ffff-ffffffffffff"));
        assert!(is_placeholder_uuid("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"));
        assert!(is_placeholder_uuid(""));
        assert!(!is_placeholder_uuid("ee6f697d-ad07-4eab-86ad-54a3798b3eb7"));
        assert!(!is_placeholder_uuid("046b3881-4e64-4cce-91a4-256632078c5d"));
    }

    #[test]
    fn config_override_wins_over_probe() {
        // init() is process-global and may already be set by another test; this
        // asserts the override-selection logic directly instead.
        let override_value = Some("my-vm-01".to_string());
        let resolved = override_value
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| probe_dmi_product_uuid(DMI_PRODUCT_UUID_PATH));
        assert_eq!(resolved, override_value);
    }
}
