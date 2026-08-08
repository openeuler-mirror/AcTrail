use plugin_system::{
    CommandExecutionContext, CommandPolicyApplyRequest, CommandPolicyApplyResult,
    CommandPolicyApplyStatus, CommandPolicyDecision, CommandPolicyListFilter,
    CommandPolicyListResult, CommandPolicyMatchDryRunRequest, CommandPolicyMatchDryRunResult,
    CommandPolicyPatchItem, CommandPolicyPatchOp, CommandPolicyRuleDraft,
};

const BINARY_VERSION: u8 = 2;
const APPLY_STATUS_ACCEPTED: u8 = 1;
const APPLY_STATUS_REJECTED: u8 = 2;

pub(super) struct CommandPolicyBinaryCodec;

impl CommandPolicyBinaryCodec {
    pub(super) fn encode_context(context: &CommandExecutionContext) -> Result<Vec<u8>, String> {
        let mut bytes = vec![BINARY_VERSION];
        Self::push_string(&mut bytes, &context.syscall)?;
        Self::push_string(&mut bytes, &context.requested_path)?;
        Self::push_string(&mut bytes, &context.resolved_path)?;
        Self::push_count(&mut bytes, context.argv.len())?;
        for arg in &context.argv {
            Self::push_string(&mut bytes, arg)?;
        }
        Self::push_option_i32(&mut bytes, context.execveat_dirfd);
        Self::push_option_u64(&mut bytes, context.execveat_flags);
        Ok(bytes)
    }

    pub(super) fn parse_list_filter(bytes: &[u8]) -> Result<CommandPolicyListFilter, String> {
        let mut cursor = BinaryCursor::new(bytes);
        cursor.require_version()?;
        let decision =
            cursor.read_option(|cursor| CommandPolicyDecision::from_code(cursor.read_u8()?))?;
        let executable_prefix = cursor.read_option_string()?;
        cursor.finish("command policy list filter")?;
        Ok(CommandPolicyListFilter {
            decision,
            executable_prefix,
        })
    }

    pub(super) fn parse_match_request(
        bytes: &[u8],
    ) -> Result<CommandPolicyMatchDryRunRequest, String> {
        let mut cursor = BinaryCursor::new(bytes);
        cursor.require_version()?;
        let executable = cursor.read_string()?;
        let args = cursor.read_string_list()?;
        cursor.finish("command policy match request")?;
        Ok(CommandPolicyMatchDryRunRequest { executable, args })
    }

    pub(super) fn parse_apply_request(bytes: &[u8]) -> Result<CommandPolicyApplyRequest, String> {
        let mut cursor = BinaryCursor::new(bytes);
        cursor.require_version()?;
        let base_revision = cursor.read_u64()?;
        let mutation_id = cursor.read_string()?;
        let reason = cursor.read_option_string()?;
        let item_count = cursor.read_count()?;
        let mut items = Vec::with_capacity(item_count);
        for _ in 0..item_count {
            items.push(Self::parse_patch_item(&mut cursor)?);
        }
        cursor.finish("command policy apply request")?;
        Ok(CommandPolicyApplyRequest {
            base_revision,
            mutation_id,
            reason,
            items,
        })
    }

    pub(super) fn encode_list_result(result: &CommandPolicyListResult) -> Result<Vec<u8>, String> {
        let mut bytes = vec![BINARY_VERSION];
        bytes.extend_from_slice(&result.source_revision.to_le_bytes());
        Self::push_option_string(&mut bytes, result.next_cursor.as_deref())?;
        Self::push_count(&mut bytes, result.rules.len())?;
        for rule in &result.rules {
            Self::push_string(&mut bytes, &rule.rule_id)?;
            Self::push_string(&mut bytes, &rule.owner_instance_id)?;
            bytes.push(rule.decision.code());
            Self::push_string(&mut bytes, &rule.executable)?;
            Self::push_option_string_list(&mut bytes, rule.args.as_deref())?;
            Self::push_option_string(&mut bytes, rule.gray_target.as_deref())?;
            bytes.extend_from_slice(&rule.priority.to_le_bytes());
            bytes.extend_from_slice(&rule.rule_revision.to_le_bytes());
            bytes.extend_from_slice(&rule.updated_sequence.to_le_bytes());
        }
        Ok(bytes)
    }

    pub(super) fn encode_match_result(
        result: &CommandPolicyMatchDryRunResult,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = vec![
            BINARY_VERSION,
            u8::from(result.matched),
            result.decision.code(),
        ];
        Self::push_option_string(&mut bytes, result.rule_id.as_deref())?;
        Self::push_option_string(&mut bytes, result.owner_instance_id.as_deref())?;
        Self::push_string(&mut bytes, &result.resolved_executable)?;
        Self::push_option_u64(&mut bytes, result.rule_revision);
        bytes.extend_from_slice(&result.source_revision.to_le_bytes());
        Ok(bytes)
    }

    pub(super) fn encode_apply_result(
        result: &CommandPolicyApplyResult,
    ) -> Result<Vec<u8>, String> {
        let status = match result.status {
            CommandPolicyApplyStatus::Accepted => APPLY_STATUS_ACCEPTED,
            CommandPolicyApplyStatus::Rejected => APPLY_STATUS_REJECTED,
        };
        let mut bytes = vec![BINARY_VERSION, status];
        bytes.extend_from_slice(&result.new_revision.to_le_bytes());
        bytes.extend_from_slice(&result.applied_count.to_le_bytes());
        bytes.extend_from_slice(&result.rejected_count.to_le_bytes());
        Self::push_count(&mut bytes, result.errors.len())?;
        for error in &result.errors {
            bytes.extend_from_slice(&error.item_index.to_le_bytes());
            Self::push_string(&mut bytes, &error.code)?;
            Self::push_string(&mut bytes, &error.message)?;
        }
        Ok(bytes)
    }

    fn parse_patch_item(cursor: &mut BinaryCursor<'_>) -> Result<CommandPolicyPatchItem, String> {
        let op = CommandPolicyPatchOp::from_code(cursor.read_u8()?)?;
        let rule_id = cursor.read_option_string()?;
        let rule = cursor.read_option(Self::parse_rule_draft)?;
        Ok(CommandPolicyPatchItem { op, rule_id, rule })
    }

    fn parse_rule_draft(cursor: &mut BinaryCursor<'_>) -> Result<CommandPolicyRuleDraft, String> {
        Ok(CommandPolicyRuleDraft {
            rule_id: cursor.read_option_string()?,
            decision: CommandPolicyDecision::from_code(cursor.read_u8()?)?,
            executable: cursor.read_string()?,
            args: cursor.read_option(BinaryCursor::read_string_list)?,
            gray_target: cursor.read_option_string()?,
            priority: cursor.read_i32()?,
        })
    }

    fn push_count(bytes: &mut Vec<u8>, count: usize) -> Result<(), String> {
        let count = u32::try_from(count)
            .map_err(|error| format!("command policy count exceeds u32: {error}"))?;
        bytes.extend_from_slice(&count.to_le_bytes());
        Ok(())
    }

    fn push_string(bytes: &mut Vec<u8>, value: &str) -> Result<(), String> {
        Self::push_count(bytes, value.len())?;
        bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    fn push_option_string(bytes: &mut Vec<u8>, value: Option<&str>) -> Result<(), String> {
        bytes.push(u8::from(value.is_some()));
        if let Some(value) = value {
            Self::push_string(bytes, value)?;
        }
        Ok(())
    }

    fn push_option_string_list(
        bytes: &mut Vec<u8>,
        value: Option<&[String]>,
    ) -> Result<(), String> {
        bytes.push(u8::from(value.is_some()));
        if let Some(values) = value {
            Self::push_count(bytes, values.len())?;
            for value in values {
                Self::push_string(bytes, value)?;
            }
        }
        Ok(())
    }

    fn push_option_i32(bytes: &mut Vec<u8>, value: Option<i32>) {
        bytes.push(u8::from(value.is_some()));
        if let Some(value) = value {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn push_option_u64(bytes: &mut Vec<u8>, value: Option<u64>) {
        bytes.push(u8::from(value.is_some()));
        if let Some(value) = value {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
}

struct BinaryCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BinaryCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn require_version(&mut self) -> Result<(), String> {
        let version = self.read_u8()?;
        if version == BINARY_VERSION {
            Ok(())
        } else {
            Err(format!("invalid command policy binary version {version}"))
        }
    }

    fn finish(&self, label: &str) -> Result<(), String> {
        (self.offset == self.bytes.len())
            .then_some(())
            .ok_or_else(|| format!("{label} has trailing bytes"))
    }

    fn read_option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        match self.read_u8()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            tag => Err(format!("invalid command policy option tag {tag}")),
        }
    }

    fn read_option_string(&mut self) -> Result<Option<String>, String> {
        self.read_option(Self::read_string)
    }

    fn read_count(&mut self) -> Result<usize, String> {
        usize::try_from(self.read_u32()?)
            .map_err(|error| format!("command policy count overflow: {error}"))
    }

    fn read_string(&mut self) -> Result<String, String> {
        let len = self.read_count()?;
        let bytes = self.read_exact(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|error| format!("command policy string is not UTF-8: {error}"))
    }

    fn read_string_list(&mut self) -> Result<Vec<String>, String> {
        let count = self.read_count()?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(self.read_string()?);
        }
        Ok(values)
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        self.read_exact(4)?
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| "command policy u32 is truncated".to_string())
    }

    fn read_u64(&mut self) -> Result<u64, String> {
        self.read_exact(8)?
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| "command policy u64 is truncated".to_string())
    }

    fn read_i32(&mut self) -> Result<i32, String> {
        self.read_exact(4)?
            .try_into()
            .map(i32::from_le_bytes)
            .map_err(|_| "command policy i32 is truncated".to_string())
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| "command policy binary offset overflow".to_string())?;
        if end > self.bytes.len() {
            return Err("command policy binary payload is truncated".to_string());
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }
}
