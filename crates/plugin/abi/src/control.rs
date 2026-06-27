pub mod context {
    pub const CURRENT_DECISION: &str = "c";
    pub const CURRENT_FILE_POLICY: &str = "f";
}

pub mod query {
    pub const DECISION_SUMMARY: &str = "decision-summary.v1";
    pub const MATCHED_RULE: &str = "matched-rule.v1";
}

pub mod file_policy_write {
    pub const SCHEMA_VERSION: &str = "file-policy-write.v1";
}

pub mod subject_code {
    pub const FILE_ACCESS: u8 = 1;
    pub const COMMAND_EXECUTION: u8 = 2;
    pub const NETWORK_ACTION: u8 = 3;
}
