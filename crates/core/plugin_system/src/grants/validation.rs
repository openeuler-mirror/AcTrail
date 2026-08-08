use model_core::payload::PayloadSourceBoundary;

use crate::{CommandPolicyDecision, FilePolicyDecision};

pub(super) struct GrantValidator;

impl GrantValidator {
    pub(super) fn file_decision(decision: FilePolicyDecision) -> Result<(), String> {
        if matches!(decision, FilePolicyDecision::Default) {
            return Err("file-policy.rules.apply grant kind cannot be default".to_string());
        }
        Ok(())
    }

    pub(super) fn command_decision(decision: CommandPolicyDecision) -> Result<(), String> {
        if decision == CommandPolicyDecision::Default {
            return Err("command-policy.rules.apply grant kind cannot be default".to_string());
        }
        Ok(())
    }

    pub(super) fn file_path_scope(path: &str) -> Result<(), String> {
        Self::policy_path_scope("file-policy.rules.apply", path)
    }

    pub(super) fn command_path_scope(path: &str) -> Result<(), String> {
        Self::policy_path_scope("command-policy.rules.apply", path)
    }

    pub(super) fn env_name(name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("env-read grant name must not be empty".to_string());
        }
        let mut chars = name.chars();
        let first = chars
            .next()
            .ok_or_else(|| "env-read grant name must not be empty".to_string())?;
        if !(first == '_' || first.is_ascii_alphabetic()) {
            return Err(format!(
                "env-read grant name {name} must start with an ASCII letter or underscore"
            ));
        }
        if chars.any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric())) {
            return Err(format!(
                "env-read grant name {name} must contain only ASCII letters, digits, and underscores"
            ));
        }
        Ok(())
    }

    pub(super) fn payload_source(source: &str) -> Result<(), String> {
        match source {
            "syscall" | "tls-user-space" | "stdio" => Ok(()),
            _ => Err(format!(
                "unsupported payload-read source {source}; expected syscall, tls-user-space, or stdio"
            )),
        }
    }

    pub(super) fn payload_source_name(source: PayloadSourceBoundary) -> &'static str {
        match source {
            PayloadSourceBoundary::Syscall => "syscall",
            PayloadSourceBoundary::TlsUserSpace => "tls-user-space",
            PayloadSourceBoundary::Stdio => "stdio",
        }
    }

    fn policy_path_scope(label: &str, path: &str) -> Result<(), String> {
        if path.is_empty() {
            return Err(format!("{label} path scope must not be empty"));
        }
        if path.contains('\0') {
            return Err(format!("{label} path scope contains NUL"));
        }
        let check_path = path.strip_suffix("/**").unwrap_or(path);
        if !check_path.starts_with('/') {
            return Err(format!("{label} path scope {path} must be absolute"));
        }
        if check_path.is_empty() {
            return Err(format!(
                "{label} recursive path scope must have an absolute base"
            ));
        }
        if check_path.contains('*') {
            return Err(format!(
                "{label} path scope {path} may only use /** as its final suffix"
            ));
        }
        Ok(())
    }
}
