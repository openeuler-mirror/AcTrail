//! Fail-fast provider-rule file loading.

use std::fs;
use std::path::Path;

use crate::rules::ProviderRule;

const RULE_KEY: &str = "rule";
const REQUIRED_RULE_PARTS: usize = 4;
const RULE_PARTS_WITH_RATIONALE: usize = 5;

pub fn load_rules(path: &Path) -> Result<Vec<ProviderRule>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read provider rules {}: {error}", path.display()))?;
    let mut rules = Vec::new();
    for (line_index, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        rules.push(parse_rule_line(line_index, trimmed)?);
    }
    if rules.is_empty() {
        return Err(format!(
            "provider rules {} must contain at least one rule",
            path.display()
        ));
    }
    Ok(rules)
}

fn parse_rule_line(line_index: usize, line: &str) -> Result<ProviderRule, String> {
    let (key, value) = line
        .split_once('=')
        .ok_or_else(|| format!("invalid provider rule line {}", line_index + 1))?;
    if key.trim() != RULE_KEY {
        return Err(format!(
            "invalid provider rule line {}: expected key {RULE_KEY}",
            line_index + 1
        ));
    }
    let parts = value
        .split('|')
        .map(|part| part.trim().to_string())
        .collect::<Vec<_>>();
    match parts.len() {
        REQUIRED_RULE_PARTS | RULE_PARTS_WITH_RATIONALE => {}
        _ => {
            return Err(format!(
                "invalid provider rule line {}: expected provider|field|equals|confidence_millis[|rationale]",
                line_index + 1
            ));
        }
    }

    let provider = required_part(line_index, &parts, RulePart::Provider)?;
    let field = required_part(line_index, &parts, RulePart::Field)?;
    let equals = required_part(line_index, &parts, RulePart::Equals)?;
    let confidence_millis = required_part(line_index, &parts, RulePart::ConfidenceMillis)?
        .parse::<u16>()
        .map_err(|error| {
            format!(
                "invalid provider rule line {} confidence_millis: {error}",
                line_index + 1
            )
        })?;
    let rationale = parts
        .get(RulePart::Rationale.index())
        .filter(|value| !value.is_empty())
        .cloned();

    Ok(ProviderRule {
        field,
        equals,
        provider,
        confidence_millis,
        rationale,
    })
}

fn required_part(line_index: usize, parts: &[String], part: RulePart) -> Result<String, String> {
    parts
        .get(part.index())
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "invalid provider rule line {}: {} must not be empty",
                line_index + 1,
                part.name()
            )
        })
}

enum RulePart {
    Provider,
    Field,
    Equals,
    ConfidenceMillis,
    Rationale,
}

impl RulePart {
    const fn index(&self) -> usize {
        match self {
            Self::Provider => 0,
            Self::Field => 1,
            Self::Equals => 2,
            Self::ConfidenceMillis => 3,
            Self::Rationale => 4,
        }
    }

    const fn name(&self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Field => "field",
            Self::Equals => "equals",
            Self::ConfidenceMillis => "confidence_millis",
            Self::Rationale => "rationale",
        }
    }
}
