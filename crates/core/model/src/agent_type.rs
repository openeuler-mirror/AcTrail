//! Agent product resolved from the executable an agent process runs.
//!
//! AcTrail observes agents from the outside, so the agent product (Codex,
//! Claude Code, xiaoo, ...) is not part of any captured event. The live
//! runtime already binds an `agent.identity` to the executable observed for
//! the same `(trace_id, process)`; this module turns that path into a product
//! name so exporters publish a conclusion instead of leaving every consumer
//! to re-implement the same path parsing.
//!
//! Resolution order, strongest first:
//!
//! 1. [`AgentTypeSource::KnownProduct`] - the path matched a known product,
//!    either by program name or by the package directory it is installed in.
//! 2. [`AgentTypeSource::Executable`] - nothing matched; the name is the
//!    program name, which still keeps different agents distinguishable.
//!
//! An interpreter-hosted agent started as `python3 /srv/app/main.py` resolves
//! to `python3`, because the argument vector is not observed. Such a value is
//! reported with the weaker [`AgentTypeSource::Executable`] source so
//! consumers can tell a match from a fallback.

use std::fmt;

/// Longest agent type name produced.
pub const AGENT_TYPE_NAME_MAX_LEN: usize = 64;

/// How an [`AgentType`] was resolved. Consumers use it to weigh the name:
/// a known product is a match, an executable name is a fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentTypeSource {
    KnownProduct,
    Executable,
}

impl AgentTypeSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KnownProduct => "known_product",
            Self::Executable => "executable",
        }
    }
}

impl fmt::Display for AgentTypeSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentType {
    pub name: String,
    pub source: AgentTypeSource,
}

impl AgentType {
    /// Classify the executable observed for an agent process.
    ///
    /// Returns `None` only when the path carries no usable name at all; a
    /// path that names an interpreter still resolves, with the weaker
    /// [`AgentTypeSource::Executable`] source.
    pub fn from_executable(executable: &str) -> Option<Self> {
        let token = executable.trim();
        if token.is_empty() {
            return None;
        }
        let program = program_name(token);
        if let Some(product) = known_product(token, program) {
            return Some(Self {
                name: product.to_string(),
                source: AgentTypeSource::KnownProduct,
            });
        }
        fallback_name(token, program).map(|name| Self {
            name,
            source: AgentTypeSource::Executable,
        })
    }
}

/// Known agent products: `(agent type, program names, package path markers)`.
///
/// Program names are compared with the basename of the path after a script
/// extension is removed. Package markers are matched as substrings of the
/// whole path and cover agents installed as a script under a package
/// directory, where the basename is only `cli.js` or `index.js`.
const KNOWN_PRODUCTS: &[(&str, &[&str], &[&str])] = &[
    ("claude-code", &["claude"], &["@anthropic-ai/claude-code"]),
    ("codex", &["codex"], &["@openai/codex"]),
    ("opencode", &["opencode"], &["opencode-ai"]),
    ("xiaoo", &["xiaoo"], &[]),
    ("qoder-cli", &["qodercli", "qoder"], &["@qoder-ai/qodercli"]),
    ("pi", &["pi"], &["@mariozechner/pi-coding-agent"]),
    ("gemini-cli", &["gemini"], &["@google/gemini-cli"]),
    ("qwen-code", &["qwen"], &["@qwen-code/qwen-code"]),
    ("cursor-agent", &["cursor-agent"], &[]),
    ("openclaw", &["openclaw"], &[]),
    ("aider", &["aider"], &[]),
    ("goose", &["goose"], &[]),
    ("kimi-cli", &["kimi"], &[]),
    ("crush", &["crush"], &[]),
];

fn known_product(token: &str, program_name: &str) -> Option<&'static str> {
    KNOWN_PRODUCTS
        .iter()
        .find(|(_, names, markers)| {
            names.contains(&program_name) || markers.iter().any(|marker| token.contains(marker))
        })
        .map(|(product, _, _)| *product)
}

fn program_name(token: &str) -> &str {
    let basename = token.rsplit('/').next().unwrap_or(token);
    for extension in [".js", ".mjs", ".cjs", ".py", ".sh", ".exe"] {
        if let Some(stem) = basename.strip_suffix(extension) {
            if !stem.is_empty() {
                return stem;
            }
        }
    }
    basename
}

/// Script names that say nothing about the agent they start.
const GENERIC_SCRIPT_NAMES: &[&str] = &["main", "index", "cli", "app", "run", "start", "__main__"];

/// Name an unrecognised program. A generic entry script such as
/// `/srv/deep_agent/main.py` is named after its directory instead.
fn fallback_name(token: &str, program_name: &str) -> Option<String> {
    if GENERIC_SCRIPT_NAMES.contains(&program_name) {
        let directory = token
            .rsplit('/')
            .nth(1)
            .filter(|directory| !matches!(*directory, "" | "." | ".." | "bin" | "src" | "dist"));
        if let Some(name) = directory.and_then(sanitize_name) {
            return Some(name);
        }
    }
    sanitize_name(program_name)
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

fn sanitize_name(program_name: &str) -> Option<String> {
    let name: String = program_name
        .bytes()
        .take(AGENT_TYPE_NAME_MAX_LEN)
        .map(|byte| {
            if is_name_byte(byte) {
                byte as char
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.bytes().all(|byte| byte == b'_') {
        None
    } else {
        Some(name)
    }
}
