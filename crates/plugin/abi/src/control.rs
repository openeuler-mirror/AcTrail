pub mod context {
    pub const CURRENT_DECISION: &str = "c";
    pub const CURRENT_FILE_POLICY: &str = "f";
}

pub mod query {
    pub const DECISION_SUMMARY: &str = "decision-summary.v1";
    pub const MATCHED_RULE: &str = "matched-rule.v1";
}

pub mod file_policy {
    pub mod decision_code {
        pub const DEFAULT: u8 = 0;
        pub const ALLOW: u8 = 1;
        pub const DENY: u8 = 2;
        pub const GRAY: u8 = 3;
    }

    pub mod operation_code {
        pub const ANY: u8 = 0;
        pub const OPEN: u8 = 1;
        pub const MKDIR: u8 = 2;
        pub const RMDIR: u8 = 3;
    }

    pub mod patch_op_code {
        pub const UPSERT: u8 = 1;
        pub const DELETE: u8 = 2;
        pub const ENABLE: u8 = 3;
        pub const DISABLE: u8 = 4;
    }

    pub mod apply_mode_code {
        pub const PARTIAL: u8 = 1;
        pub const AON: u8 = 2;
    }

    pub mod apply_status_code {
        pub const ACCEPTED: u8 = 1;
        pub const REJECTED: u8 = 2;
    }

    pub mod grant {
        pub const CURRENT_MATCH_GET: &str = "file-access.current-match-get";
        pub const CURRENT_CONTEXT_QUERY: &str = "file-access.current-context-query";
        pub const RULES_READ: &str = "file-policy.rules.read";
        pub const RULES_MATCH_DRY_RUN: &str = "file-policy.rules.match-dry-run";
        pub const RULES_VALIDATE: &str = "file-policy.rules.validate";
        pub const RULES_APPLY: &str = "file-policy.rules.apply";
    }
}

pub mod subject_code {
    pub const FILE_ACCESS: u8 = 1;
    pub const COMMAND_EXECUTION: u8 = 2;
    pub const NETWORK_ACTION: u8 = 3;
}
