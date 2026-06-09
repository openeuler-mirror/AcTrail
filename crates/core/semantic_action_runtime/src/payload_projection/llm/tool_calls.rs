//! Tool call extraction and SSE delta assembly.

use serde_json::{Map, Number, Value};

const TOOL_CALL_PARENT_KEYS: &[&str] = &["message", "delta", "choices", "output"];

pub(super) fn collect_tool_call_deltas_json(value: &Value) -> Option<String> {
    let mut calls = Vec::new();
    collect_tool_call_delta_values(value, &mut calls);
    (!calls.is_empty()).then(|| Value::Array(calls).to_string())
}

pub(super) fn assembled_tool_calls_json(value: &Value) -> Option<String> {
    let mut assembler = ToolCallAssembler::default();
    assembler.apply_value(value);
    assembler.into_json()
}

pub(super) fn assembled_tool_calls_json_from_values<'a>(
    values: impl IntoIterator<Item = &'a Value>,
) -> Option<String> {
    let mut assembler = ToolCallAssembler::default();
    for value in values {
        assembler.apply_value(value);
    }
    assembler.into_json()
}

fn collect_tool_call_delta_values(value: &Value, calls: &mut Vec<Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_tool_call_delta_values(item, calls);
            }
        }
        Value::Object(object) => {
            if let Some(Value::Array(tool_calls)) = object.get("tool_calls") {
                calls.extend(tool_calls.iter().cloned());
            }
            for key in TOOL_CALL_PARENT_KEYS {
                if let Some(child) = object.get(*key) {
                    collect_tool_call_delta_values(child, calls);
                }
            }
        }
        _ => {}
    }
}

#[derive(Default)]
struct ToolCallAssembler {
    calls: Vec<ToolCallAccumulator>,
}

impl ToolCallAssembler {
    fn apply_value(&mut self, value: &Value) {
        match value {
            Value::Array(items) => {
                for item in items {
                    self.apply_value(item);
                }
            }
            Value::Object(object) => {
                if let Some(Value::Array(tool_calls)) = object.get("tool_calls") {
                    for tool_call in tool_calls {
                        if let Value::Object(tool_call) = tool_call {
                            self.apply_tool_call_delta(tool_call);
                        }
                    }
                }
                for key in TOOL_CALL_PARENT_KEYS {
                    if let Some(child) = object.get(*key) {
                        self.apply_value(child);
                    }
                }
            }
            _ => {}
        }
    }

    fn apply_tool_call_delta(&mut self, delta: &Map<String, Value>) {
        let index = delta.get("index").and_then(Value::as_u64);
        let id = delta.get("id").and_then(Value::as_str);
        let Some(call) = self.call_slot(index, id) else {
            return;
        };
        if let Some(index) = index {
            call.index.get_or_insert(index);
        }
        if let Some(id) = id.and_then(non_empty) {
            call.id.get_or_insert_with(|| id.to_string());
        }
        if let Some(kind) = delta
            .get("type")
            .and_then(Value::as_str)
            .and_then(non_empty)
        {
            call.kind = Some(kind.to_string());
        }
        if let Some(function) = delta.get("function").and_then(Value::as_object) {
            call.apply_function_delta(function);
        }
    }

    fn call_slot(
        &mut self,
        index: Option<u64>,
        id: Option<&str>,
    ) -> Option<&mut ToolCallAccumulator> {
        let has_id = id.and_then(non_empty).is_some();
        if index.is_none() && !has_id {
            return None;
        }
        if let Some(position) = self.calls.iter().position(|call| {
            index.is_some_and(|index| call.index == Some(index))
                || id
                    .and_then(non_empty)
                    .is_some_and(|id| call.id.as_deref() == Some(id))
        }) {
            return self.calls.get_mut(position);
        }
        self.calls.push(ToolCallAccumulator {
            index,
            id: id.and_then(non_empty).map(ToString::to_string),
            ..ToolCallAccumulator::default()
        });
        self.calls.last_mut()
    }

    fn into_json(self) -> Option<String> {
        let calls = self
            .calls
            .into_iter()
            .filter_map(ToolCallAccumulator::into_value)
            .collect::<Vec<_>>();
        (!calls.is_empty()).then(|| Value::Array(calls).to_string())
    }
}

#[derive(Default)]
struct ToolCallAccumulator {
    index: Option<u64>,
    id: Option<String>,
    kind: Option<String>,
    function_name: Option<String>,
    function_arguments: String,
    has_function_arguments: bool,
}

impl ToolCallAccumulator {
    fn apply_function_delta(&mut self, function: &Map<String, Value>) {
        if let Some(name) = function
            .get("name")
            .and_then(Value::as_str)
            .and_then(non_empty)
        {
            self.function_name = Some(name.to_string());
        }
        if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
            self.function_arguments.push_str(arguments);
            self.has_function_arguments = true;
        }
    }

    fn into_value(self) -> Option<Value> {
        let function = self.function_value();
        let mut object = Map::new();
        if let Some(index) = self.index {
            object.insert("index".to_string(), Value::Number(Number::from(index)));
        }
        if let Some(id) = self.id {
            object.insert("id".to_string(), Value::String(id));
        }
        if let Some(kind) = self.kind {
            object.insert("type".to_string(), Value::String(kind));
        }
        if let Some(function) = function {
            object.insert("function".to_string(), function);
        }
        (!object.is_empty()).then(|| Value::Object(object))
    }

    fn function_value(&self) -> Option<Value> {
        let mut function = Map::new();
        if let Some(name) = &self.function_name {
            function.insert("name".to_string(), Value::String(name.clone()));
        }
        if self.has_function_arguments {
            function.insert(
                "arguments".to_string(),
                Value::String(self.function_arguments.clone()),
            );
            if let Ok(arguments_json) = serde_json::from_str::<Value>(&self.function_arguments) {
                function.insert("arguments_json".to_string(), arguments_json);
            }
        }
        (!function.is_empty()).then(|| Value::Object(function))
    }
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}
