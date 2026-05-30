//! Stable JSON serialization boundary for exported graphs.

use std::fmt::Write;

use graph_contract::document::{GraphDocument, GraphEdge, GraphNode};

const JSON_INDENT_UNIT: &str = "    ";

pub fn to_json(document: &GraphDocument) -> String {
    let mut output = String::new();
    write_document(&mut output, document);
    output.push('\n');
    output
}

fn write_document(output: &mut String, document: &GraphDocument) {
    output.push('{');
    write_indent(output, 1);
    write_json_field(output, "schema_version", &document.schema_version);
    output.push(',');
    write_indent(output, 1);
    write_json_field(output, "trace_id", &document.trace_id.to_string());
    output.push(',');
    write_indent(output, 1);
    write_json_field(
        output,
        "completeness",
        &format!("{:?}", document.completeness),
    );
    output.push(',');
    write_indent(output, 1);
    output.push_str("\"nodes\":");
    write_nodes(output, &document.nodes, 1);
    output.push(',');
    write_indent(output, 1);
    output.push_str("\"edges\":");
    write_edges(output, &document.edges, 1);
    write_indent(output, 0);
    output.push('}');
}

fn write_nodes(output: &mut String, nodes: &[GraphNode], depth: usize) {
    output.push('[');
    for (index, node) in nodes.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write_indent(output, depth + 1);
        output.push('{');
        write_indent(output, depth + 2);
        write_json_field(output, "id", &node.id);
        output.push(',');
        write_indent(output, depth + 2);
        write_json_field(output, "kind", &format!("{:?}", node.kind));
        output.push(',');
        write_indent(output, depth + 2);
        write_json_field(output, "title", &node.title);
        output.push(',');
        write_indent(output, depth + 2);
        write_attributes(output, &node.attributes, depth + 2);
        write_indent(output, depth + 1);
        output.push('}');
    }
    if !nodes.is_empty() {
        write_indent(output, depth);
    }
    output.push(']');
}

fn write_edges(output: &mut String, edges: &[GraphEdge], depth: usize) {
    output.push('[');
    for (index, edge) in edges.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write_indent(output, depth + 1);
        output.push('{');
        write_indent(output, depth + 2);
        write_json_field(output, "from", &edge.from);
        output.push(',');
        write_indent(output, depth + 2);
        write_json_field(output, "to", &edge.to);
        output.push(',');
        write_indent(output, depth + 2);
        write_json_field(output, "relationship", &format!("{:?}", edge.relationship));
        output.push(',');
        write_indent(output, depth + 2);
        write_attributes(output, &edge.attributes, depth + 2);
        write_indent(output, depth + 1);
        output.push('}');
    }
    if !edges.is_empty() {
        write_indent(output, depth);
    }
    output.push(']');
}

fn write_attributes(
    output: &mut String,
    attributes: &std::collections::BTreeMap<String, String>,
    depth: usize,
) {
    output.push_str("\"attributes\":{");
    for (index, (key, value)) in attributes.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write_indent(output, depth + 1);
        write_json_field(output, key, value);
    }
    if !attributes.is_empty() {
        write_indent(output, depth);
    }
    output.push('}');
}

fn write_json_field(output: &mut String, key: &str, value: &str) {
    let _ = write!(output, "\"{}\":\"{}\"", escape(key), escape(value));
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            character if character <= '\u{1f}' => {
                let _ = write!(escaped, "\\u{:04x}", character as u32);
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn write_indent(output: &mut String, depth: usize) {
    output.push('\n');
    for _ in 0..depth {
        output.push_str(JSON_INDENT_UNIT);
    }
}
