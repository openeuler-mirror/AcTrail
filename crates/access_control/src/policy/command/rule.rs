use super::args::CommandArgsPattern;
use super::path::CommandExecutablePath;
use super::validation::CommandRuleDraftValidator;
use crate::policy::path::PolicyPath;
use plugin_system::{
    CommandPolicyDecision, CommandPolicyListFilter, CommandPolicyRuleDraft, CommandPolicyRuleView,
};
use std::path::PathBuf;

pub(super) const STATIC_POLICY_OWNER: &str = "actrail.static";
const GENERATED_RULE_ID_PREFIX: &str = "command-rule";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredCommandRule {
    pub owner_instance_id: String,
    pub rule_id: String,
    pub decision: CommandPolicyDecision,
    pub(super) executable: PathBuf,
    pub(super) execution_executable: PathBuf,
    pub(super) args: CommandArgsPattern,
    pub gray_target: Option<String>,
    pub(super) priority: i32,
    pub rule_revision: u64,
    pub(super) updated_sequence: u64,
}

impl StoredCommandRule {
    pub fn args_view(&self) -> Option<Vec<String>> {
        self.args.view()
    }

    pub(super) fn from_draft(
        owner_instance_id: &str,
        item_rule_id: Option<&str>,
        draft: &CommandPolicyRuleDraft,
        rule_revision: u64,
        updated_sequence: u64,
        generated_rule_id: u64,
    ) -> Result<Self, String> {
        let rule_id = item_rule_id
            .map(str::to_string)
            .or_else(|| draft.rule_id.clone())
            .unwrap_or_else(|| format!("{GENERATED_RULE_ID_PREFIX}-{generated_rule_id}"));
        CommandRuleDraftValidator::validate_id(&rule_id)?;
        CommandRuleDraftValidator::validate_shape(draft)?;
        let executable = CommandExecutablePath::normalize_absolute(&draft.executable)?;
        let execution_executable = PolicyPath::bind(std::path::Path::new(&draft.executable))?;
        Ok(Self {
            owner_instance_id: owner_instance_id.to_string(),
            rule_id,
            decision: draft.decision,
            executable,
            execution_executable,
            args: CommandArgsPattern::parse(draft.args.as_deref())?,
            gray_target: draft.gray_target.clone(),
            priority: draft.priority,
            rule_revision,
            updated_sequence,
        })
    }

    pub(super) fn view(&self) -> CommandPolicyRuleView {
        CommandPolicyRuleView {
            rule_id: self.rule_id.clone(),
            owner_instance_id: self.owner_instance_id.clone(),
            decision: self.decision,
            executable: self.executable.display().to_string(),
            args: self.args.view(),
            gray_target: self.gray_target.clone(),
            priority: self.priority,
            rule_revision: self.rule_revision,
            updated_sequence: self.updated_sequence,
        }
    }

    pub(super) fn matches_filter(&self, filter: &CommandPolicyListFilter) -> bool {
        filter
            .decision
            .is_none_or(|decision| decision == self.decision)
            && filter.executable_prefix.as_ref().is_none_or(|prefix| {
                self.executable
                    .display()
                    .to_string()
                    .starts_with(prefix.as_str())
            })
    }
}
