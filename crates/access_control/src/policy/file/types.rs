use config_core::daemon::EnforcementDecision;
use plugin_system::FilePolicyDecision;
use std::path::PathBuf;

pub(super) const BUILTIN_POLICY_OWNER: &str = "actrail.builtin";
pub(super) const STATIC_POLICY_OWNER: &str = "actrail.static";
pub(super) const HOST_RULE_ID_PREFIX: &str = "fp";
pub(super) const DEFAULT_GRAY_TIMEOUT_MS: u64 = 30_000;
pub(super) const DEFAULT_GRAY_CONCURRENCY_LIMIT: u32 = 32;
pub(super) const DEFAULT_GRAY_FALLBACK: EnforcementDecision = EnforcementDecision::Deny;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnforcementRule {
    pub owner_instance_id: String,
    pub rule_id: String,
    pub decision: RuleDecision,
    pub operation: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltinIgnoreMark {
    pub path: PathBuf,
    pub recursive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleDecision {
    Local(EnforcementDecision),
    SyncPlugin {
        instance_id: String,
        timeout_ms: u64,
        concurrency_limit: u32,
        fallback: EnforcementDecision,
    },
}

impl RuleDecision {
    pub(super) fn from_policy(
        decision: FilePolicyDecision,
        gray_target: Option<u64>,
    ) -> Result<RuleDecision, String> {
        match decision {
            FilePolicyDecision::Allow => Ok(RuleDecision::Local(EnforcementDecision::Allow)),
            FilePolicyDecision::Deny => Ok(RuleDecision::Local(EnforcementDecision::Deny)),
            FilePolicyDecision::Gray => {
                let Some(target) = gray_target else {
                    return Err("gray rule requires gray_target".to_string());
                };
                Ok(RuleDecision::SyncPlugin {
                    instance_id: target.to_string(),
                    timeout_ms: DEFAULT_GRAY_TIMEOUT_MS,
                    concurrency_limit: DEFAULT_GRAY_CONCURRENCY_LIMIT,
                    fallback: DEFAULT_GRAY_FALLBACK,
                })
            }
            FilePolicyDecision::Default => {
                Err("file policy rule decision cannot be default".to_string())
            }
        }
    }
}
