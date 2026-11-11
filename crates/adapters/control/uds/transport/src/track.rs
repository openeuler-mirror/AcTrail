//! Track registration wire fields.
use super::{
    ControlCodecError, decode_binary_identity, decode_process_ref, encode_process_ref, field,
    parse_bool, parse_u64, parse_usize,
};
use control_contract::command::{
    ControlCommand, LaunchTarget, LaunchTlsProbePlan, TrackAddCommand,
};
use model_core::ids::{ProfileName, RequestId, TraceName};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(super) struct TrackCodec;
impl TrackCodec {
    pub(super) fn encode(fields: &mut Vec<String>, command: &TrackAddCommand) {
        fields.push("track_add_v9".to_string());
        fields.push(command.request_id.get().to_string());
        encode_process_ref(fields, &command.root);
        fields.push(command.display_name.to_string());
        fields.push(command.profile_name.to_string());
        fields.push(command.launch_mode.to_string());
        fields.push(command.tls_probe_plans.len().to_string());
        for plan in &command.tls_probe_plans {
            fields.push(plan.target.display().to_string());
            fields.push(plan.target_identity.identity_type_code.code().to_string());
            fields.push(plan.target_identity.identity.clone());
            fields.push(plan.binary.display().to_string());
            fields.push(plan.binary_identity.identity_type_code.code().to_string());
            fields.push(plan.binary_identity.identity.clone());
            fields.push(plan.provider.clone());
            fields.push(plan.points.clone());
        }
        fields.push(command.launch_target.is_some().to_string());
        if let Some(target) = &command.launch_target {
            fields.push(target.executable.display().to_string());
            fields.push(target.candidates.len().to_string());
            fields.extend(
                target
                    .candidates
                    .iter()
                    .map(|path| path.display().to_string()),
            );
        }
        fields.push(command.tags.len().to_string());
        fields.extend(command.tags.iter().cloned());
        super::checkpoint::encode_launch_config(fields, command.checkpoint.as_ref());
    }
    pub(super) fn decode(fields: &[String]) -> Result<ControlCommand, ControlCodecError> {
        let request_id = RequestId::new(parse_u64(field(fields, 1)?, "request_id")?);
        let root = decode_process_ref(fields, 2)?;
        let display_name = TraceName::new(field(fields, 4)?);
        let profile_name = ProfileName::new(field(fields, 5)?);
        let launch_mode = parse_bool(field(fields, 6)?, "launch_mode")?;
        let plan_count = parse_usize(field(fields, 7)?, "tls_probe_plan_count")?;
        let mut cursor = 8;
        let mut tls_probe_plans = Vec::new();
        for _ in 0..plan_count {
            tls_probe_plans.push(LaunchTlsProbePlan {
                target: PathBuf::from(field(fields, cursor)?),
                target_identity: decode_binary_identity(fields, cursor + 1, "target")?,
                binary: PathBuf::from(field(fields, cursor + 3)?),
                binary_identity: decode_binary_identity(fields, cursor + 4, "binary")?,
                provider: field(fields, cursor + 6)?.clone(),
                points: field(fields, cursor + 7)?.clone(),
            });
            cursor += 8;
        }
        let present = parse_bool(field(fields, cursor)?, "launch_target_present")?;
        cursor += 1;
        let launch_target = if present {
            let executable = PathBuf::from(field(fields, cursor)?);
            let count = parse_usize(field(fields, cursor + 1)?, "launch_candidate_count")?;
            cursor += 2;
            let mut candidates = Vec::new();
            for _ in 0..count {
                candidates.push(PathBuf::from(field(fields, cursor)?));
                cursor += 1;
            }
            Some(LaunchTarget {
                executable,
                candidates,
            })
        } else {
            None
        };
        let tag_count = parse_usize(field(fields, cursor)?, "tag_count")?;
        cursor += 1;
        let mut tags = BTreeSet::new();
        for offset in 0..tag_count {
            tags.insert(field(fields, cursor + offset)?.clone());
        }
        let checkpoint = super::checkpoint::decode_launch_config(fields, cursor + tag_count)?.0;
        Ok(ControlCommand::TrackAdd(TrackAddCommand {
            request_id,
            root,
            display_name,
            profile_name,
            tags,
            launch_mode,
            launch_target,
            tls_probe_plans,
            checkpoint,
        }))
    }
}
