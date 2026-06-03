//! Equal-length rewrite processor used by the sync probe MVP.

use crate::{CoreError, CoreResult, Decision, PayloadContext, PayloadDirection, SyncProcessor};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewriteRule {
    direction: PayloadDirection,
    from: Vec<u8>,
    to: Vec<u8>,
    label: String,
}

impl RewriteRule {
    pub fn new(
        direction: PayloadDirection,
        from: Vec<u8>,
        to: Vec<u8>,
        label: impl Into<String>,
    ) -> CoreResult<Self> {
        if from.is_empty() {
            return Err(CoreError::new("rewrite rule match bytes must not be empty"));
        }
        if from.len() != to.len() {
            return Err(CoreError::new(format!(
                "rewrite rule must be equal length, from={} to={}",
                from.len(),
                to.len()
            )));
        }
        Ok(Self {
            direction,
            from,
            to,
            label: label.into(),
        })
    }

    pub fn direction(&self) -> PayloadDirection {
        self.direction
    }

    pub fn from(&self) -> &[u8] {
        &self.from
    }

    pub fn to(&self) -> &[u8] {
        &self.to
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Debug, Default)]
pub struct EqualLenRewriteProcessor {
    rules: Vec<RewriteRule>,
}

impl EqualLenRewriteProcessor {
    pub fn new(rules: Vec<RewriteRule>) -> Self {
        Self { rules }
    }
}

impl SyncProcessor for EqualLenRewriteProcessor {
    fn decide(&mut self, context: &PayloadContext<'_>, payload: &[u8]) -> CoreResult<Decision> {
        let mut replacement = payload.to_vec();
        let mut matched = Vec::new();
        for rule in &self.rules {
            if rule.direction() != context.direction {
                continue;
            }
            if replace_all(&mut replacement, rule.from(), rule.to()) {
                matched.push(rule.label().to_string());
            }
        }
        if matched.is_empty() {
            Ok(Decision::Allow)
        } else {
            Ok(Decision::ReplaceEqualLen {
                replacement,
                reason: matched.join(","),
            })
        }
    }
}

fn replace_all(payload: &mut [u8], from: &[u8], to: &[u8]) -> bool {
    let mut matched = false;
    let mut offset = 0;
    while let Some(index) = find_bytes(&payload[offset..], from) {
        let start = offset + index;
        let end = start + from.len();
        payload[start..end].copy_from_slice(to);
        offset = end;
        matched = true;
    }
    matched
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_len_processor_rewrites_matching_direction() {
        let rule = RewriteRule::new(
            PayloadDirection::Outbound,
            "你好".as_bytes().to_vec(),
            b"Hello!".to_vec(),
            "demo",
        )
        .expect("valid equal length rule");
        let mut processor = EqualLenRewriteProcessor::new(vec![rule]);
        let context = PayloadContext {
            direction: PayloadDirection::Outbound,
            provider: "openssl",
            symbol: "SSL_write",
            stream_key: 1,
        };
        let decision = processor
            .decide(&context, "say 你好".as_bytes())
            .expect("processor decision");

        assert_eq!(
            decision,
            Decision::ReplaceEqualLen {
                replacement: b"say Hello!".to_vec(),
                reason: "demo".to_string()
            }
        );
    }

    #[test]
    fn rewrite_rule_rejects_length_change() {
        let error = RewriteRule::new(
            PayloadDirection::Outbound,
            "你好".as_bytes().to_vec(),
            b"Good".to_vec(),
            "bad",
        )
        .expect_err("length-changing rule should fail");

        assert!(error.to_string().contains("equal length"));
    }
}
