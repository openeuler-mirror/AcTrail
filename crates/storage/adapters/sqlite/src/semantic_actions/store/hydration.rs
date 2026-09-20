use std::collections::HashMap;
use std::sync::OnceLock;

use rusqlite::Connection;
use semantic_action::{
    SemanticAction, SemanticActionKind as Kind, SemanticActionLinkRole as Role,
    SemanticActionStoreError, SemanticEvidence, attr_keys as attrs,
};

use crate::semantic_actions::codebook::sqlite::{
    decode_evidence_kind, decode_link_role, link_role_code,
};

/// Builds the public read view from independently persisted facts.
pub(in crate::semantic_actions) struct ActionReadHydrator;

impl ActionReadHydrator {
    pub(super) fn remove_relationship_attributes(action: &mut SemanticAction) {
        let keys: &[&str] = match action.kind {
            Kind::LlmCall => &[
                attrs::llm_call::REQUEST_ACTION_ID,
                attrs::llm_call::RESPONSE_ACTION_ID,
                attrs::llm_call::HTTP_RESPONSE_ACTION_ID,
            ],
            Kind::LlmToolCall => &[attrs::llm_tool_call::RESPONSE_ACTION_ID],
            Kind::LlmToolResult => &[attrs::llm_tool_result::REQUEST_ACTION_ID],
            Kind::HttpMessage | Kind::LlmResponse => &[attrs::http_response::REQUEST_ACTION_ID],
            Kind::SseEvent => &[attrs::sse::STREAM_ACTION_ID],
            Kind::McpToolCall
            | Kind::McpRequest
            | Kind::McpResponse
            | Kind::McpStdin
            | Kind::McpStdout => &[
                attrs::mcp::TOOL_CALL_ACTION_ID,
                attrs::mcp::REQUEST_ACTION_ID,
                attrs::mcp::RESPONSE_ACTION_ID,
                attrs::mcp::STDIN_ACTION_ID,
                attrs::mcp::STDOUT_ACTION_ID,
            ],
            _ => &[],
        };
        for key in keys {
            action.attributes.remove(*key);
        }
    }

    pub(in crate::semantic_actions) fn hydrate<'a>(
        connection: &Connection,
        actions: impl IntoIterator<Item = &'a mut SemanticAction>,
        include_evidence: bool,
    ) -> Result<(), SemanticActionStoreError> {
        let mut actions: Vec<_> = actions.into_iter().collect();
        if actions.is_empty() {
            return Ok(());
        }
        let mut ids: Vec<_> = actions.iter().map(|action| &action.action_id).collect();
        ids.sort_unstable();
        ids.dedup();
        let selected = serde_json::to_string(&ids).map_err(Self::error)?;
        let mut indexes = HashMap::<String, Vec<usize>>::new();
        for (index, action) in actions.iter_mut().enumerate() {
            Self::remove_relationship_attributes(action);
            indexes
                .entry(action.action_id.clone())
                .or_default()
                .push(index);
            if include_evidence {
                action.evidence.clear();
            }
        }
        if include_evidence {
            Self::hydrate_evidence(connection, &selected, &indexes, &mut actions)?;
        }
        let relation_ids: Vec<_> = actions
            .iter()
            .filter(|action| {
                matches!(
                    action.kind,
                    Kind::LlmCall
                        | Kind::LlmToolCall
                        | Kind::LlmToolResult
                        | Kind::LlmResponse
                        | Kind::HttpMessage
                        | Kind::SseEvent
                        | Kind::McpToolCall
                        | Kind::McpRequest
                        | Kind::McpResponse
                        | Kind::McpStdin
                        | Kind::McpStdout
                )
            })
            .map(|action| &action.action_id)
            .collect();
        if relation_ids.is_empty() {
            return Ok(());
        }
        let selected_relations = serde_json::to_string(&relation_ids).map_err(Self::error)?;
        let edges = Self::read_relationships(connection, &selected_relations)?;
        for edge in &edges {
            if let Some((id, key, value)) = edge.attribute() {
                if let Some(indices) = indexes.get(id) {
                    for index in indices {
                        actions[*index]
                            .attributes
                            .insert(key.to_owned(), value.to_owned());
                    }
                }
            }
        }
        let http_requests: HashMap<_, _> = edges
            .iter()
            .filter(|edge| edge.role == Role::HttpRequestHttpResponse)
            .map(|edge| (edge.child.as_str(), edge.parent.as_str()))
            .collect();
        let request_http: HashMap<_, _> = edges
            .iter()
            .filter(|edge| edge.role == Role::LlmRequestHttpMessage)
            .map(|edge| (edge.parent.as_str(), edge.child.as_str()))
            .collect();
        let call_requests: HashMap<_, _> = edges
            .iter()
            .filter(|edge| edge.role == Role::LlmCallRequest)
            .map(|edge| (edge.parent.as_str(), edge.child.as_str()))
            .collect();
        for edge in edges
            .iter()
            .filter(|edge| edge.role == Role::LlmCallResponse)
        {
            let request = call_requests
                .get(edge.parent.as_str())
                .and_then(|id| request_http.get(id));
            if let (Some(indices), Some(request)) = (indexes.get(&edge.child), request) {
                for index in indices {
                    actions[*index].attributes.insert(
                        attrs::http_response::REQUEST_ACTION_ID.to_owned(),
                        (*request).to_owned(),
                    );
                }
            }
        }
        for edge in &edges {
            if edge.role != Role::LlmResponseHttpMessage {
                continue;
            }
            if let (Some(indices), Some(request)) = (
                indexes.get(&edge.parent),
                http_requests.get(edge.child.as_str()),
            ) {
                for index in indices {
                    actions[*index].attributes.insert(
                        attrs::http_response::REQUEST_ACTION_ID.to_owned(),
                        (*request).to_owned(),
                    );
                }
            }
        }
        let mut groups = HashMap::<String, McpRelations>::new();
        for edge in &edges {
            match edge.role {
                Role::McpToolCallRequest => {
                    groups.entry(edge.parent.clone()).or_default().request =
                        Some(edge.child.clone())
                }
                Role::McpToolCallResponse => {
                    groups.entry(edge.parent.clone()).or_default().response =
                        Some(edge.child.clone())
                }
                _ => {}
            }
        }
        let messages: HashMap<_, _> = groups
            .iter()
            .flat_map(|(root, group)| {
                group
                    .request
                    .iter()
                    .chain(group.response.iter())
                    .map(move |id| (id.clone(), root.clone()))
            })
            .collect();
        for edge in &edges {
            if let Some(group) = messages
                .get(&edge.parent)
                .and_then(|root| groups.get_mut(root))
            {
                match edge.role {
                    Role::McpRequestStdout => group.stdout = Some(edge.child.clone()),
                    Role::McpResponseStdin => group.stdin = Some(edge.child.clone()),
                    _ => {}
                }
            }
        }
        for (root, group) in groups {
            for id in std::iter::once(&root)
                .chain(group.request.iter())
                .chain(group.response.iter())
                .chain(group.stdout.iter())
                .chain(group.stdin.iter())
            {
                if let Some(indices) = indexes.get(id) {
                    for index in indices {
                        group.hydrate(&root, actions[*index]);
                    }
                }
            }
        }
        Ok(())
    }

    fn hydrate_evidence(
        connection: &Connection,
        selected: &str,
        indexes: &HashMap<String, Vec<usize>>,
        actions: &mut [&mut SemanticAction],
    ) -> Result<(), SemanticActionStoreError> {
        let mut statement = connection
            .prepare_cached(
                "SELECT ids.action_id, evidence.kind_code, evidence.evidence_id, evidence.role
             FROM json_each(?1) selected
             JOIN semantic_action_ids ids ON ids.action_id = selected.value
             JOIN semantic_action_evidence evidence ON evidence.action_key = ids.action_key
             ORDER BY evidence.evidence_key",
            )
            .map_err(Self::error)?;
        let rows = statement
            .query_map([selected], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    SemanticEvidence {
                        kind: decode_evidence_kind(row.get(1)?)?,
                        id: row.get(2)?,
                        role: row.get(3)?,
                    },
                ))
            })
            .map_err(Self::error)?;
        for row in rows {
            let (id, evidence) = row.map_err(Self::error)?;
            if let Some(indices) = indexes.get(&id) {
                for index in indices {
                    actions[*index].evidence.push(evidence.clone());
                }
            }
        }
        Ok(())
    }

    fn read_relationships(
        connection: &Connection,
        selected: &str,
    ) -> Result<Vec<Relation>, SemanticActionStoreError> {
        static SQL: OnceLock<String> = OnceLock::new();
        let sql = SQL.get_or_init(|| format!(
            "WITH selected AS (
                SELECT DISTINCT ids.trace_id, ids.action_key FROM json_each(?1) item
                JOIN semantic_action_ids ids ON ids.action_id = item.value
             ), mcp_parents AS (
                SELECT trace_id, action_key FROM selected
                UNION SELECT link.trace_id, link.parent_action_key FROM selected
                JOIN semantic_action_links link ON selected.trace_id = link.trace_id AND selected.action_key = link.child_action_key
                WHERE link.valid = 1 AND link.role_code IN ({request}, {response}, {stdout}, {stdin})
             ), roots AS (
                SELECT trace_id, action_key FROM mcp_parents
                UNION SELECT link.trace_id, link.parent_action_key FROM mcp_parents
                JOIN semantic_action_links link ON mcp_parents.trace_id = link.trace_id AND mcp_parents.action_key = link.child_action_key
                WHERE link.valid = 1 AND link.role_code IN ({request}, {response})
             ), mcp_messages AS (
                SELECT link.trace_id, link.child_action_key AS action_key FROM roots
                JOIN semantic_action_links link ON roots.trace_id = link.trace_id AND roots.action_key = link.parent_action_key
                WHERE link.valid = 1 AND link.role_code IN ({request}, {response})
             ), response_requests AS (
                SELECT request.trace_id, request.parent_action_key, request.child_action_key, request.role_code
                FROM selected
                JOIN semantic_action_links response ON response.trace_id = selected.trace_id AND response.child_action_key = selected.action_key
                    AND response.valid = 1 AND response.role_code = {call_response}
                JOIN semantic_action_links request ON request.trace_id = response.trace_id AND request.parent_action_key = response.parent_action_key
                    AND request.valid = 1 AND request.role_code = {call_request}
             ), wanted AS (
                SELECT link.parent_action_key, link.child_action_key, link.role_code
                FROM selected JOIN semantic_action_links link ON link.trace_id = selected.trace_id AND link.parent_action_key = selected.action_key
                WHERE link.valid = 1
                UNION SELECT link.parent_action_key, link.child_action_key, link.role_code
                FROM selected JOIN semantic_action_links link ON link.trace_id = selected.trace_id AND link.child_action_key = selected.action_key
                WHERE link.valid = 1
                UNION SELECT link.parent_action_key, link.child_action_key, link.role_code
                FROM roots JOIN semantic_action_links link ON link.trace_id = roots.trace_id AND link.parent_action_key = roots.action_key
                WHERE link.valid = 1 AND link.role_code IN ({request}, {response})
                UNION SELECT link.parent_action_key, link.child_action_key, link.role_code
                FROM mcp_messages JOIN semantic_action_links link ON link.trace_id = mcp_messages.trace_id AND link.parent_action_key = mcp_messages.action_key
                WHERE link.valid = 1 AND link.role_code IN ({stdout}, {stdin})
                UNION SELECT exchange.parent_action_key, exchange.child_action_key, exchange.role_code
                FROM selected
                JOIN semantic_action_links response ON response.trace_id = selected.trace_id AND response.parent_action_key = selected.action_key
                    AND response.valid = 1 AND response.role_code = {llm_http_response}
                JOIN semantic_action_links exchange ON exchange.trace_id = response.trace_id AND exchange.child_action_key = response.child_action_key
                    AND exchange.valid = 1 AND exchange.role_code = {http_exchange}
                UNION SELECT parent_action_key, child_action_key, role_code FROM response_requests
                UNION SELECT link.parent_action_key, link.child_action_key, link.role_code
                FROM response_requests
                JOIN semantic_action_links link ON link.trace_id = response_requests.trace_id AND link.parent_action_key = response_requests.child_action_key
                    AND link.valid = 1 AND link.role_code = {request_http}
             ) SELECT parent.action_id, child.action_id, wanted.role_code FROM wanted
             JOIN semantic_action_ids parent ON parent.action_key = wanted.parent_action_key
             JOIN semantic_action_ids child ON child.action_key = wanted.child_action_key",
            request = link_role_code(Role::McpToolCallRequest),
            response = link_role_code(Role::McpToolCallResponse),
            stdout = link_role_code(Role::McpRequestStdout),
            stdin = link_role_code(Role::McpResponseStdin),
            llm_http_response = link_role_code(Role::LlmResponseHttpMessage),
            http_exchange = link_role_code(Role::HttpRequestHttpResponse),
            call_response = link_role_code(Role::LlmCallResponse),
            call_request = link_role_code(Role::LlmCallRequest),
            request_http = link_role_code(Role::LlmRequestHttpMessage),
        ));
        let mut statement = connection.prepare_cached(sql).map_err(Self::error)?;
        statement
            .query_map([selected], |row| {
                Ok(Relation {
                    parent: row.get(0)?,
                    child: row.get(1)?,
                    role: decode_link_role(row.get(2)?)?,
                })
            })
            .map_err(Self::error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Self::error)
    }

    fn error(error: impl std::fmt::Display) -> SemanticActionStoreError {
        SemanticActionStoreError::new("hydrate_semantic_action", error.to_string())
    }
}

struct Relation {
    parent: String,
    child: String,
    role: Role,
}

impl Relation {
    fn attribute(&self) -> Option<(&str, &'static str, &str)> {
        let (on_parent, key) = match self.role {
            Role::LlmCallRequest => (true, attrs::llm_call::REQUEST_ACTION_ID),
            Role::LlmCallResponse => (true, attrs::llm_call::RESPONSE_ACTION_ID),
            Role::LlmCallHttpResponse => (true, attrs::llm_call::HTTP_RESPONSE_ACTION_ID),
            Role::LlmResponseToolCall => (false, attrs::llm_tool_call::RESPONSE_ACTION_ID),
            Role::LlmRequestToolResult => (false, attrs::llm_tool_result::REQUEST_ACTION_ID),
            Role::HttpRequestHttpResponse => (false, attrs::http_response::REQUEST_ACTION_ID),
            Role::SseStreamEvent => (false, attrs::sse::STREAM_ACTION_ID),
            _ => return None,
        };
        Some(if on_parent {
            (&self.parent, key, &self.child)
        } else {
            (&self.child, key, &self.parent)
        })
    }
}

#[derive(Default)]
struct McpRelations {
    request: Option<String>,
    response: Option<String>,
    stdout: Option<String>,
    stdin: Option<String>,
}

impl McpRelations {
    fn hydrate(&self, root: &str, action: &mut SemanticAction) {
        if action.kind != Kind::McpToolCall {
            action
                .attributes
                .insert(attrs::mcp::TOOL_CALL_ACTION_ID.to_owned(), root.to_owned());
        }
        for (key, value) in [
            (attrs::mcp::REQUEST_ACTION_ID, self.request.as_ref()),
            (attrs::mcp::STDOUT_ACTION_ID, self.stdout.as_ref()),
        ] {
            if let Some(value) = value {
                action.attributes.insert(key.to_owned(), value.clone());
            }
        }
        if matches!(
            action.kind,
            Kind::McpToolCall | Kind::McpResponse | Kind::McpStdin
        ) {
            for (key, value) in [
                (attrs::mcp::RESPONSE_ACTION_ID, self.response.as_ref()),
                (attrs::mcp::STDIN_ACTION_ID, self.stdin.as_ref()),
            ] {
                if let Some(value) = value {
                    action.attributes.insert(key.to_owned(), value.clone());
                }
            }
        }
    }
}
