use std::{borrow::Cow, fmt};

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::llm_pipeline::provider::LlmRequestFacts;

use super::strict::Discard;

macro_rules! scalar_defaults {
    () => {
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
    };
}

/// Per-request facts only. Text is borrowed until the final role of a message
/// is known; duplicate fields follow Value's last-value-wins semantics.
#[derive(Default)]
pub(super) struct SelectedRequest<'a> {
    pub(super) facts: LlmRequestFacts,
    pub(super) system_parts: Vec<Cow<'a, str>>,
}

impl<'a> SelectedRequest<'a> {
    pub(super) fn parse(bytes: &'a [u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

impl<'de> Deserialize<'de> for SelectedRequest<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(RequestVisitor)
    }
}

struct RequestVisitor;
impl<'de> Visitor<'de> for RequestVisitor {
    type Value = SelectedRequest<'de>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON request")
    }
    scalar_defaults!();
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(Default::default())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        Discard::sequence(&mut sequence)?;
        Ok(Default::default())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut selected = SelectedRequest::default();
        let facts = &mut selected.facts;
        let mut context_fields = [false; 4];
        let mut tools_present = false;
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            match key.as_ref() {
                "model" => {
                    facts.model_present = true;
                    facts.model = map
                        .next_value::<TextNode>()?
                        .into_string()
                        .map(Cow::into_owned);
                }
                "model_name" => {
                    facts.model_name_present = true;
                    facts.model_name = map
                        .next_value::<TextNode>()?
                        .into_string()
                        .map(Cow::into_owned);
                }
                "provider_model_name" => {
                    facts.provider_model_name = map
                        .next_value::<TextNode>()?
                        .into_string()
                        .map(Cow::into_owned);
                }
                "messages" => {
                    facts.input_present = true;
                    let messages = map.next_value::<Messages>()?;
                    facts.messages_have_text = messages.have_text;
                    selected.system_parts = messages.system_parts;
                }
                "prompt" | "input" => {
                    facts.input_present = true;
                    map.next_value::<Discard>()?;
                }
                "tools" => {
                    tools_present = true;
                    map.next_value::<Discard>()?;
                }
                "user_input" | "session_id" | "conversation_id" | "config_name" => {
                    let index = match key.as_ref() {
                        "user_input" => 0,
                        "session_id" => 1,
                        "conversation_id" => 2,
                        _ => 3,
                    };
                    context_fields[index] = map.next_value::<TextNode>()?.nonempty_string();
                }
                _ => {
                    map.next_value::<Discard>()?;
                }
            }
        }
        facts.context_signal = tools_present || context_fields.into_iter().any(|value| value);
        Ok(selected)
    }
}

#[derive(Default)]
struct Messages<'a> {
    have_text: bool,
    system_parts: Vec<Cow<'a, str>>,
}

impl<'de> Deserialize<'de> for Messages<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(MessagesVisitor)
    }
}

struct MessagesVisitor;
impl<'de> Visitor<'de> for MessagesVisitor {
    type Value = Messages<'de>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("request messages")
    }
    scalar_defaults!();
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(Default::default())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        Discard::map(&mut map)?;
        Ok(Default::default())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut result = Messages::default();
        while let Some(mut message) = sequence.next_element::<Message>()? {
            result.have_text |= message.have_text;
            result.system_parts.append(&mut message.system_parts);
        }
        Ok(result)
    }
}

#[derive(Default)]
struct Message<'a> {
    have_text: bool,
    system_parts: Vec<Cow<'a, str>>,
}

impl<'de> Deserialize<'de> for Message<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(MessageVisitor)
    }
}

struct MessageVisitor;
impl<'de> Visitor<'de> for MessageVisitor {
    type Value = Message<'de>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a request message")
    }
    scalar_defaults!();
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(Default::default())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        Discard::sequence(&mut sequence)?;
        Ok(Default::default())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut role = None;
        let mut content = None;
        let mut text = TextNode::default();
        let mut input = TextNode::default();
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            match key.as_ref() {
                "role" => role = map.next_value::<TextNode>()?.into_string(),
                "content" => content = Some(map.next_value::<TextNode>()?),
                "text" => text = map.next_value::<TextNode>()?,
                "input" => input = map.next_value::<TextNode>()?,
                _ => {
                    map.next_value::<Discard>()?;
                }
            }
        }
        let have_text = role.is_some() && content.as_ref().is_some_and(TextNode::has_text);
        let system_parts = if role.as_deref() == Some("system") {
            if let Some(content) = content {
                content.parts
            } else {
                text.parts.append(&mut input.parts);
                text.parts
            }
        } else {
            Vec::new()
        };
        Ok(Message {
            have_text,
            system_parts,
        })
    }
}

/// Only text/content/input descendants are candidates for background metadata.
/// Other objects (including tool arguments and result metadata) are discarded.
#[derive(Default)]
struct TextNode<'a> {
    parts: Vec<Cow<'a, str>>,
    is_string: bool,
    array_has_text: bool,
    text_block: bool,
}

impl<'a> TextNode<'a> {
    fn into_string(mut self) -> Option<Cow<'a, str>> {
        self.is_string.then(|| self.parts.pop()).flatten()
    }
    fn nonempty_string(&self) -> bool {
        self.is_string && self.parts.first().is_some_and(|text| !text.is_empty())
    }
    fn has_text(&self) -> bool {
        self.nonempty_string() || self.array_has_text
    }
    fn text(text: Cow<'a, str>) -> Self {
        Self {
            parts: vec![text],
            is_string: true,
            ..Self::default()
        }
    }
}

impl<'de> Deserialize<'de> for TextNode<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(TextVisitor)
    }
}

struct TextVisitor;
impl<'de> Visitor<'de> for TextVisitor {
    type Value = TextNode<'de>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    scalar_defaults!();
    fn visit_borrowed_str<E: serde::de::Error>(self, text: &'de str) -> Result<Self::Value, E> {
        Ok(TextNode::text(Cow::Borrowed(text)))
    }
    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
        Ok(TextNode::text(Cow::Owned(text.to_owned())))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut node = TextNode::default();
        while let Some(mut child) = sequence.next_element::<TextNode>()? {
            node.array_has_text |= child.text_block;
            node.parts.append(&mut child.parts);
        }
        Ok(node)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut text_type = false;
        let mut text = TextNode::default();
        let mut content = TextNode::default();
        let mut input = TextNode::default();
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            match key.as_ref() {
                "type" => {
                    text_type =
                        map.next_value::<TextNode>()?.into_string().as_deref() == Some("text")
                }
                "text" => text = map.next_value::<TextNode>()?,
                "content" => content = map.next_value::<TextNode>()?,
                "input" => input = map.next_value::<TextNode>()?,
                _ => {
                    map.next_value::<Discard>()?;
                }
            }
        }
        let text_block = text_type && text.nonempty_string();
        text.parts.append(&mut content.parts);
        text.parts.append(&mut input.parts);
        Ok(TextNode {
            parts: text.parts,
            text_block,
            ..Default::default()
        })
    }
}
