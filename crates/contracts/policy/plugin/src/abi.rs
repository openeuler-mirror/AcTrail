//! Plugin-facing ABI contracts for policy extensions.

use policy_evaluate_contract::decision::PolicyDecision;
use policy_evaluate_contract::evaluate::PolicyInput;

use crate::manifest::PluginManifest;

pub trait PolicyPlugin {
    fn manifest(&self) -> &PluginManifest;
    fn evaluate(&self, input: &PolicyInput) -> PolicyDecision;
}
