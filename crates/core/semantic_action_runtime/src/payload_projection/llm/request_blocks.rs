use std::collections::BTreeMap;

use model_core::ids::TraceId;
use semantic_action::{
    LlmRequestBlock, LlmRequestBlockRef, LlmRequestContentWrite, LlmRequestManifest,
};
use serde_json::{Map, Number, Value};

use self::canonical_json::{
    bytes as canonical_json_bytes, sha256_hex, string as canonical_json_string,
};
use self::metadata::{background_request_kind, message_preview, user_message_metadata};

mod canonical_json;
mod metadata;

pub(super) use self::metadata::UserMessageMetadata;

pub(super) const FORMAT_VERSION: u32 = 2;

const BLOCK_PLACEHOLDER_KEY: &str = "$actrail_llm_block";

pub(super) struct CanonicalRequestContent {
    pub(super) write: LlmRequestContentWrite,
    pub(crate) trajectory_history: Option<TrajectoryHistoryProjection>,
    pub(super) canonical_body_hash: String,
    pub(super) canonical_body_bytes: u64,
    pub(super) block_count: usize,
    pub(super) message_preview: Option<String>,
    pub(super) user_message_count: usize,
    pub(super) latest_user_message_hash: Option<String>,
    pub(super) background_kind: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct HistoryAtom {
    structural_json: Option<String>,
    block_hashes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TrajectoryHistoryProjection {
    Supported(Vec<HistoryAtom>),
    UnsupportedMultimodal,
}

pub(super) fn canonical_request_content(
    trace_id: TraceId,
    action_id: &str,
    body: &Value,
    project_trajectory_history: bool,
) -> Result<CanonicalRequestContent, String> {
    let user_messages = user_message_metadata(body);
    let canonical_body = canonical_json_bytes(body);
    let canonical_body_hash = sha256_hex(&canonical_body);
    let canonical_body_bytes = canonical_body.len() as u64;
    let mut accumulator = BlockAccumulator::new(trace_id, action_id);
    let (skeleton, trajectory_history) =
        skeletonize_body(body, &mut accumulator, project_trajectory_history)?;
    let skeleton_json = canonical_json_string(&skeleton);
    let (block_refs, blocks) = accumulator.into_parts();
    let manifest = LlmRequestManifest {
        trace_id,
        action_id: action_id.to_string(),
        format_version: FORMAT_VERSION,
        canonical_body_hash: canonical_body_hash.clone(),
        canonical_body_bytes,
        skeleton_json,
    };
    let block_count = block_refs.len();
    Ok(CanonicalRequestContent {
        write: LlmRequestContentWrite {
            manifest,
            block_refs,
            blocks,
        },
        trajectory_history,
        canonical_body_hash,
        canonical_body_bytes,
        block_count,
        message_preview: message_preview(body),
        user_message_count: user_messages.count,
        latest_user_message_hash: user_messages.latest_hash,
        background_kind: background_request_kind(body),
    })
}

pub(super) fn canonical_shape_metadata(
    body: &Value,
) -> (
    String,
    u64,
    Option<String>,
    UserMessageMetadata,
    Option<&'static str>,
) {
    let canonical_body = canonical_json_bytes(body);
    (
        sha256_hex(&canonical_body),
        canonical_body.len() as u64,
        message_preview(body),
        user_message_metadata(body),
        background_request_kind(body),
    )
}

fn skeletonize_body(
    body: &Value,
    accumulator: &mut BlockAccumulator,
    project_trajectory_history: bool,
) -> Result<(Value, Option<TrajectoryHistoryProjection>), String> {
    let Some(object) = body.as_object() else {
        return Ok((
            body.clone(),
            project_trajectory_history.then(|| TrajectoryHistoryProjection::Supported(Vec::new())),
        ));
    };
    let mut skeleton = Map::new();
    let mut messages_history = None;
    let mut input_history = None;
    let mut prompt_history = None;
    for (key, value) in object {
        let next = match key.as_str() {
            "tools" => skeletonize_array_items(value, accumulator)?,
            "messages" => {
                let projected =
                    skeletonize_messages(value, accumulator, project_trajectory_history)?;
                messages_history = projected.history;
                projected.skeleton
            }
            "prompt" => {
                if project_trajectory_history {
                    let block = accumulator.add_block_with_hash(value)?;
                    let block_hash = block
                        .block_hash
                        .ok_or_else(|| "trajectory prompt block hash is missing".to_string())?;
                    prompt_history = Some(history_for_block(value, block_hash));
                    block.placeholder
                } else {
                    accumulator.add_block(value)?
                }
            }
            "input" => {
                let projected = skeletonize_input(value, accumulator, project_trajectory_history)?;
                input_history = projected.history;
                projected.skeleton
            }
            _ => value.clone(),
        };
        skeleton.insert(key.clone(), next);
    }
    let history = messages_history
        .or(input_history)
        .or(prompt_history)
        .or_else(|| {
            project_trajectory_history.then(|| TrajectoryHistoryProjection::Supported(Vec::new()))
        });
    Ok((Value::Object(skeleton), history))
}

struct SkeletonizedHistory {
    skeleton: Value,
    history: Option<TrajectoryHistoryProjection>,
}

fn skeletonize_array_items(
    value: &Value,
    accumulator: &mut BlockAccumulator,
) -> Result<Value, String> {
    let Some(items) = value.as_array() else {
        return accumulator.add_block(value);
    };
    items
        .iter()
        .map(|item| accumulator.add_block(item))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn skeletonize_messages(
    value: &Value,
    accumulator: &mut BlockAccumulator,
    project_history: bool,
) -> Result<SkeletonizedHistory, String> {
    let mut projector = MessageProjector::new(accumulator, project_history);
    let Some(messages) = value.as_array() else {
        let projected = projector.skeletonize(value)?;
        return Ok(SkeletonizedHistory {
            skeleton: projected.skeleton,
            history: projected.atom.map(|atom| {
                if projected.multimodal {
                    TrajectoryHistoryProjection::UnsupportedMultimodal
                } else {
                    TrajectoryHistoryProjection::Supported(vec![atom])
                }
            }),
        });
    };
    let mut skeleton = Vec::with_capacity(messages.len());
    let mut atoms = project_history.then(|| Vec::with_capacity(messages.len()));
    let mut multimodal = false;
    for message in messages {
        let projected = projector.skeletonize(message)?;
        skeleton.push(projected.skeleton);
        if let (Some(atoms), Some(atom)) = (&mut atoms, projected.atom) {
            atoms.push(atom);
        }
        multimodal |= projected.multimodal;
    }
    Ok(SkeletonizedHistory {
        skeleton: Value::Array(skeleton),
        history: atoms.map(|atoms| {
            if multimodal {
                TrajectoryHistoryProjection::UnsupportedMultimodal
            } else {
                TrajectoryHistoryProjection::Supported(atoms)
            }
        }),
    })
}

struct SkeletonizedMessage {
    skeleton: Value,
    atom: Option<HistoryAtom>,
    multimodal: bool,
}

struct MessageProjector<'a> {
    accumulator: &'a mut BlockAccumulator,
    project_history: bool,
}

struct SkeletonizedMessageContent {
    skeleton: Value,
    descriptors: Option<Vec<Value>>,
    block_hashes: Vec<String>,
    multimodal: bool,
}

struct SkeletonizedContentItem {
    skeleton: Value,
    descriptor: Option<Value>,
    block_hash: Option<String>,
}

impl<'a> MessageProjector<'a> {
    fn new(accumulator: &'a mut BlockAccumulator, project_history: bool) -> Self {
        Self {
            accumulator,
            project_history,
        }
    }

    fn skeletonize(&mut self, message: &Value) -> Result<SkeletonizedMessage, String> {
        let Some(object) = message.as_object() else {
            return self.skeletonize_whole_message(message);
        };
        let Some(content) = object.get("content") else {
            return self.skeletonize_whole_message(message);
        };
        let mut skeleton = object.clone();
        let content = self.skeletonize_content(content)?;
        skeleton.insert("content".to_string(), content.skeleton);
        Ok(SkeletonizedMessage {
            skeleton: Value::Object(skeleton),
            atom: content.descriptors.map(|descriptors| HistoryAtom {
                structural_json: Some(message_structural_json(object, descriptors)),
                block_hashes: content.block_hashes,
            }),
            multimodal: content.multimodal,
        })
    }

    fn skeletonize_whole_message(
        &mut self,
        message: &Value,
    ) -> Result<SkeletonizedMessage, String> {
        let block = self
            .accumulator
            .add_block_retaining_hash(message, self.project_history)?;
        Ok(SkeletonizedMessage {
            skeleton: block.placeholder,
            atom: block.block_hash.map(HistoryAtom::from_block_hash),
            multimodal: contains_multimodal_content(message),
        })
    }

    fn skeletonize_content(
        &mut self,
        content: &Value,
    ) -> Result<SkeletonizedMessageContent, String> {
        let items = content
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_else(|| std::slice::from_ref(content));
        let mut skeleton = Vec::with_capacity(items.len());
        let mut descriptors = self
            .project_history
            .then(|| Vec::with_capacity(items.len()));
        let mut block_hashes = Vec::with_capacity(items.len());
        let mut multimodal = false;
        for item in items {
            let projected = self.skeletonize_item(item)?;
            skeleton.push(projected.skeleton);
            if let (Some(descriptors), Some(descriptor)) = (&mut descriptors, projected.descriptor)
            {
                descriptors.push(descriptor);
            }
            if let Some(block_hash) = projected.block_hash {
                block_hashes.push(block_hash);
            }
            multimodal |= contains_multimodal_content(item);
        }
        let skeleton = if content.is_array() {
            Value::Array(skeleton)
        } else {
            skeleton
                .pop()
                .ok_or_else(|| "single message content block is missing".to_string())?
        };
        Ok(SkeletonizedMessageContent {
            skeleton,
            descriptors,
            block_hashes,
            multimodal,
        })
    }

    fn skeletonize_item(&mut self, item: &Value) -> Result<SkeletonizedContentItem, String> {
        if item.is_string() {
            return self.skeletonize_nested(item, None, text_descriptor());
        }
        let Some(object) = item.as_object() else {
            return self.skeletonize_whole_item(item);
        };
        if typed_text_payload_key(object).is_some() {
            return self.skeletonize_nested(
                item,
                Some("text"),
                stable_content_descriptor(object, "text"),
            );
        }
        if block_is_tool_result(item) && object.contains_key("content") {
            return self.skeletonize_nested(
                item,
                Some("content"),
                stable_content_descriptor(object, "content"),
            );
        }
        self.skeletonize_whole_item(item)
    }

    fn skeletonize_nested(
        &mut self,
        item: &Value,
        payload_key: Option<&str>,
        descriptor: Value,
    ) -> Result<SkeletonizedContentItem, String> {
        let payload = payload_key.and_then(|key| item.get(key)).unwrap_or(item);
        let block = self
            .accumulator
            .add_block_retaining_hash(payload, self.project_history)?;
        let skeleton = if let Some(payload_key) = payload_key {
            let mut skeleton = item
                .as_object()
                .cloned()
                .ok_or_else(|| "nested content payload must belong to an object".to_string())?;
            skeleton.insert(payload_key.to_string(), block.placeholder);
            Value::Object(skeleton)
        } else {
            block.placeholder
        };
        Ok(SkeletonizedContentItem {
            skeleton,
            descriptor: self.project_history.then_some(descriptor),
            block_hash: block.block_hash,
        })
    }

    fn skeletonize_whole_item(&mut self, item: &Value) -> Result<SkeletonizedContentItem, String> {
        let block = self
            .accumulator
            .add_block_retaining_hash(item, self.project_history)?;
        Ok(SkeletonizedContentItem {
            skeleton: block.placeholder,
            descriptor: self
                .project_history
                .then(|| Value::String("block".to_string())),
            block_hash: block.block_hash,
        })
    }
}

fn typed_text_payload_key(object: &Map<String, Value>) -> Option<&'static str> {
    object
        .get("type")
        .and_then(Value::as_str)
        .filter(|kind| matches!(*kind, "text" | "input_text" | "output_text"))
        .and_then(|_| object.contains_key("text").then_some("text"))
}

fn text_descriptor() -> Value {
    let mut descriptor = Map::new();
    descriptor.insert("type".to_string(), Value::String("text".to_string()));
    Value::Object(descriptor)
}

fn stable_content_descriptor(object: &Map<String, Value>, payload_key: &str) -> Value {
    Value::Object(stable_content_fields(object, Some(payload_key)))
}

fn stable_content_fields(
    object: &Map<String, Value>,
    payload_key: Option<&str>,
) -> Map<String, Value> {
    object
        .iter()
        .filter(|(key, _)| key.as_str() != "cache_control" && Some(key.as_str()) != payload_key)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn message_structural_json(object: &Map<String, Value>, descriptors: Vec<Value>) -> String {
    let mut envelope = Map::new();
    for key in ["role", "name", "tool_call_id", "type"] {
        if let Some(value) = object.get(key) {
            envelope.insert(key.to_string(), value.clone());
        }
    }
    let mut structural = Map::new();
    structural.insert("envelope".to_string(), Value::Object(envelope));
    structural.insert("content".to_string(), Value::Array(descriptors));
    canonical_json_string(&Value::Object(structural))
}

fn skeletonize_input(
    value: &Value,
    accumulator: &mut BlockAccumulator,
    project_history: bool,
) -> Result<SkeletonizedHistory, String> {
    let items = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value));
    let mut skeleton = Vec::with_capacity(items.len());
    let mut atoms = project_history.then(|| Vec::with_capacity(items.len()));
    let mut multimodal = false;
    let mut projector = MessageProjector::new(accumulator, project_history);
    for item in items {
        let projected = projector.skeletonize(item)?;
        skeleton.push(projected.skeleton);
        if let (Some(atoms), Some(atom)) = (&mut atoms, projected.atom) {
            atoms.push(atom);
        }
        multimodal |= projected.multimodal;
    }
    let skeleton = if value.is_array() {
        Value::Array(skeleton)
    } else {
        skeleton
            .pop()
            .ok_or_else(|| "single input block is missing".to_string())?
    };
    Ok(SkeletonizedHistory {
        skeleton,
        history: atoms.map(|atoms| {
            if multimodal {
                TrajectoryHistoryProjection::UnsupportedMultimodal
            } else {
                TrajectoryHistoryProjection::Supported(atoms)
            }
        }),
    })
}

fn history_for_block(value: &Value, block_hash: String) -> TrajectoryHistoryProjection {
    if contains_multimodal_content(value) {
        TrajectoryHistoryProjection::UnsupportedMultimodal
    } else {
        TrajectoryHistoryProjection::Supported(vec![HistoryAtom::from_block_hash(block_hash)])
    }
}

impl HistoryAtom {
    fn from_block_hash(block_hash: String) -> Self {
        Self {
            structural_json: None,
            block_hashes: vec![block_hash],
        }
    }

    pub(crate) fn fits_limits(&self, max_blocks: usize, max_structural_bytes: usize) -> bool {
        self.block_hashes.len() <= max_blocks
            && self
                .structural_json
                .as_ref()
                .is_none_or(|value| value.len() <= max_structural_bytes)
    }
}

fn contains_multimodal_content(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_multimodal_content),
        Value::Object(object) => {
            object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "image"
                            | "image_url"
                            | "input_image"
                            | "audio"
                            | "input_audio"
                            | "file"
                            | "input_file"
                            | "document"
                            | "input_document"
                            | "video"
                            | "input_video"
                            | "inline_data"
                            | "file_data"
                    )
                })
                || ["image_url", "inline_data", "file_data", "source"]
                    .iter()
                    .any(|key| object.contains_key(*key))
                || object
                    .get("content")
                    .is_some_and(contains_multimodal_content)
        }
        _ => false,
    }
}

struct BlockAccumulator {
    trace_id: TraceId,
    action_id: String,
    refs: Vec<LlmRequestBlockRef>,
    blocks: BTreeMap<String, LlmRequestBlock>,
}

impl BlockAccumulator {
    fn new(trace_id: TraceId, action_id: &str) -> Self {
        Self {
            trace_id,
            action_id: action_id.to_string(),
            refs: Vec::new(),
            blocks: BTreeMap::new(),
        }
    }

    fn add_block(&mut self, value: &Value) -> Result<Value, String> {
        self.record_block(value, false)
            .map(|(placeholder, _)| placeholder)
    }

    fn add_block_with_hash(&mut self, value: &Value) -> Result<RecordedBlock, String> {
        self.record_block(value, true)
            .and_then(|(placeholder, block_hash)| {
                block_hash.map_or_else(
                    || Err("trajectory block hash is missing".to_string()),
                    |block_hash| {
                        Ok(RecordedBlock {
                            placeholder,
                            block_hash: Some(block_hash),
                        })
                    },
                )
            })
    }

    fn add_block_retaining_hash(
        &mut self,
        value: &Value,
        retain_hash: bool,
    ) -> Result<RecordedBlock, String> {
        if retain_hash {
            self.add_block_with_hash(value)
        } else {
            self.record_block(value, false)
                .map(|(placeholder, _)| RecordedBlock {
                    placeholder,
                    block_hash: None,
                })
        }
    }

    fn record_block(
        &mut self,
        value: &Value,
        retain_hash: bool,
    ) -> Result<(Value, Option<String>), String> {
        let ordinal = u32::try_from(self.refs.len())
            .map_err(|_| "LLM request block ordinal exceeds u32".to_string())?;
        let encoded_bytes = canonical_json_bytes(value);
        let block_hash = sha256_hex(&encoded_bytes);
        let block = LlmRequestBlock {
            trace_id: self.trace_id,
            block_hash: block_hash.clone(),
            uncompressed_bytes: encoded_bytes.len() as u64,
            encoded_bytes,
        };
        if let Some(existing) = self.blocks.get(&block_hash) {
            if existing != &block {
                return Err(format!(
                    "LLM request block hash collision for {}",
                    block_hash
                ));
            }
        } else {
            self.blocks.insert(block_hash.clone(), block);
        }
        let retained_hash = retain_hash.then(|| block_hash.clone());
        self.refs.push(LlmRequestBlockRef {
            trace_id: self.trace_id,
            action_id: self.action_id.clone(),
            ordinal,
            block_hash,
        });
        Ok((block_placeholder(ordinal), retained_hash))
    }

    fn into_parts(self) -> (Vec<LlmRequestBlockRef>, Vec<LlmRequestBlock>) {
        (self.refs, self.blocks.into_values().collect())
    }
}

struct RecordedBlock {
    placeholder: Value,
    block_hash: Option<String>,
}

fn block_placeholder(ordinal: u32) -> Value {
    let mut object = Map::new();
    object.insert(
        BLOCK_PLACEHOLDER_KEY.to_string(),
        Value::Number(Number::from(ordinal)),
    );
    Value::Object(object)
}

fn block_is_tool_result(block: &Value) -> bool {
    block
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "tool_result" | "tool-result"))
}
