use model_core::{host_identity::HostBootId, ids::TraceId};
use serde::{Deserialize, Deserializer, Serializer};
pub(super) struct TraceIdentity;
impl TraceIdentity {
    pub fn serialize<S: Serializer>(id: &TraceId, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(id.get())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<TraceId, D::Error> {
        Ok(TraceId::new(u64::deserialize(d)?))
    }
}
pub(super) struct BootIdentity;
impl BootIdentity {
    pub fn serialize<S: Serializer>(id: &HostBootId, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&id.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<HostBootId, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
