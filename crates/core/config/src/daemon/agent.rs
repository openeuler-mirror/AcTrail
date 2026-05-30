//! Agent semantic-action configuration.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentInvocationConfig {
    pub enabled: bool,
    pub commands: Vec<String>,
}
