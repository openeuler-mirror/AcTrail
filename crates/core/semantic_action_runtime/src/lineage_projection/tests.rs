use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessMembership};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus, SemanticEvidence,
    SemanticEvidenceKind,
};

use super::derive_lineage_links;

const TRACE_ID: TraceId = TraceId::new(7);

#[test]
fn derives_command_link_through_transient_helper_process() {
    let agent = ProcessIdentity::new(100, 1000, 1000);
    let bash = ProcessIdentity::new(101, 1001, 1001);
    let helper = ProcessIdentity::new(102, 1002, 1002);
    let base64 = ProcessIdentity::new(103, 1003, 1003);
    let memberships = vec![
        ProcessMembership::root(TRACE_ID, agent.clone(), time(0)),
        ProcessMembership::inherited(TRACE_ID, bash.clone(), agent.clone(), time(1)),
        ProcessMembership::inherited(TRACE_ID, helper.clone(), bash.clone(), time(2)),
        ProcessMembership::inherited(TRACE_ID, base64.clone(), helper, time(3)),
    ];
    let actions = vec![
        observed_agent_exec("agent-exec", agent.clone(), time(0)),
        command("agent-command", agent, time(0), None),
        command("bash-command", bash, time(1), None),
        command("base64-command", base64, time(4), None),
    ];

    let links = derive_lineage_links(TRACE_ID, &memberships, &actions, &[]);

    assert!(links.iter().any(|link| {
        link.parent_action_id == "bash-command"
            && link.child_action_id == "base64-command"
            && link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
            && link.confidence == SemanticActionLinkConfidence::Derived
    }));
    assert!(links.iter().any(|link| {
        link.parent_action_id == "agent-command"
            && link.child_action_id == "bash-command"
            && link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
    }));
    assert!(links.iter().any(|link| {
        link.parent_action_id == "agent-exec"
            && link.child_action_id == "bash-command"
            && link.role == SemanticActionLinkRole::AgentPerformedAction
    }));
    assert!(!links.iter().any(|link| {
        link.parent_action_id == "agent-exec"
            && link.child_action_id == "base64-command"
            && link.role == SemanticActionLinkRole::AgentPerformedAction
    }));
}

#[test]
fn observed_parent_link_blocks_lineage_derived_parent() {
    let parent = ProcessIdentity::new(200, 2000, 2000);
    let child = ProcessIdentity::new(201, 2001, 2001);
    let memberships = vec![
        ProcessMembership::root(TRACE_ID, parent.clone(), time(0)),
        ProcessMembership::inherited(TRACE_ID, child.clone(), parent.clone(), time(1)),
    ];
    let actions = vec![
        command("parent-command", parent, time(0), None),
        command("child-command", child, time(1), None),
    ];
    let observed = SemanticActionLink {
        trace_id: TRACE_ID,
        parent_action_id: "existing-parent".to_string(),
        child_action_id: "child-command".to_string(),
        role: SemanticActionLinkRole::CommandContainsCommandInvocation,
        confidence: SemanticActionLinkConfidence::Observed,
        valid: true,
        evidence: Vec::new(),
        attributes: BTreeMap::new(),
    };

    let links = derive_lineage_links(TRACE_ID, &memberships, &actions, &[observed]);

    assert!(!links.iter().any(|link| {
        link.parent_action_id == "parent-command"
            && link.child_action_id == "child-command"
            && link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
    }));
}

#[test]
fn stale_lineage_link_is_invalidated_when_parent_changes() {
    let parent = ProcessIdentity::new(300, 3000, 3000);
    let child = ProcessIdentity::new(301, 3001, 3001);
    let memberships = vec![
        ProcessMembership::root(TRACE_ID, parent.clone(), time(0)),
        ProcessMembership::inherited(TRACE_ID, child.clone(), parent.clone(), time(1)),
    ];
    let actions = vec![
        command("parent-command", parent, time(0), None),
        command("child-command", child, time(1), None),
    ];
    let stale = lineage_link("stale-parent", "child-command");

    let links = derive_lineage_links(TRACE_ID, &memberships, &actions, &[stale]);

    assert!(links.iter().any(|link| {
        link.parent_action_id == "stale-parent"
            && link.child_action_id == "child-command"
            && !link.valid
    }));
    assert!(links.iter().any(|link| {
        link.parent_action_id == "parent-command"
            && link.child_action_id == "child-command"
            && link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
    }));
}

fn observed_agent_exec(
    action_id: &str,
    process: ProcessIdentity,
    start_time: SystemTime,
) -> SemanticAction {
    let mut attributes = BTreeMap::new();
    attributes.insert("agent.identity.status".to_string(), "observed".to_string());
    action(
        action_id,
        SemanticActionKind::ProcessExec,
        process,
        start_time,
        None,
        attributes,
    )
}

fn command(
    action_id: &str,
    process: ProcessIdentity,
    start_time: SystemTime,
    end_time: Option<SystemTime>,
) -> SemanticAction {
    action(
        action_id,
        SemanticActionKind::CommandInvocation,
        process,
        start_time,
        end_time,
        BTreeMap::new(),
    )
}

fn action(
    action_id: &str,
    kind: SemanticActionKind,
    process: ProcessIdentity,
    start_time: SystemTime,
    end_time: Option<SystemTime>,
    attributes: BTreeMap<String, String>,
) -> SemanticAction {
    SemanticAction {
        action_id: action_id.to_string(),
        trace_id: TRACE_ID,
        kind,
        title: action_id.to_string(),
        start_time,
        end_time,
        process,
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: vec![SemanticEvidence {
            kind: SemanticEvidenceKind::Event,
            id: 1,
            role: "test".to_string(),
        }],
    }
}

fn lineage_link(parent: &str, child: &str) -> SemanticActionLink {
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "actrail.link.source".to_string(),
        "process_lineage".to_string(),
    );
    SemanticActionLink {
        trace_id: TRACE_ID,
        parent_action_id: parent.to_string(),
        child_action_id: child.to_string(),
        role: SemanticActionLinkRole::CommandContainsCommandInvocation,
        confidence: SemanticActionLinkConfidence::Derived,
        valid: true,
        evidence: Vec::new(),
        attributes,
    }
}

fn time(offset: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(offset)
}
