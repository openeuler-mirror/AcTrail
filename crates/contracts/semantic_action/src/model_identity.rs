//! Validation for provider model identifiers carried by semantic actions.

const MAX_MODEL_IDENTIFIER_BYTES: usize = 512;

/// Return a normalized model identifier when the value is safe to use as an identity key.
///
/// Provider identifiers commonly contain path, version, deployment, and fine-tune separators.
/// JSON fragments, quoted values, whitespace, and control characters are not identifiers.
pub fn validated_model_identifier(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_MODEL_IDENTIFIER_BYTES {
        return None;
    }

    let mut has_alphanumeric = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            has_alphanumeric = true;
            continue;
        }
        if matches!(character, '-' | '_' | '.' | '/' | ':' | '@' | '+') {
            continue;
        }
        return None;
    }
    has_alphanumeric.then_some(value)
}
