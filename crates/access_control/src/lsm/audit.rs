use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Clone, Debug)]
pub struct LsmAuditEvent {
    pub revision: u64,
    pub trace_id: u64,
    pub observed_ktime_ns: u64,
    pub start_boottime_ns: u64,
    pub kernel_tgid: u32,
    pub observer_tgid: u32,
    pub domain: u32,
    pub decision: u32,
    pub error: i32,
    pub signal_result: i32,
    pub rule_id: String,
    pub owner: String,
    pub resource: String,
    pub retain_audit: bool,
}

impl LsmAuditEvent {
    pub(super) fn decode(raw: &[u8]) -> Result<Self, String> {
        if raw.len() != 832 {
            return Err(format!(
                "LSM audit record has {} bytes, expected 832",
                raw.len()
            ));
        }
        let u64_at = |offset| u64::from_ne_bytes(raw[offset..offset + 8].try_into().unwrap());
        let u32_at = |offset| u32::from_ne_bytes(raw[offset..offset + 4].try_into().unwrap());
        let string = |bytes: &[u8]| {
            String::from_utf8_lossy(bytes.split(|byte| *byte == 0).next().unwrap_or_default())
                .into_owned()
        };
        let domain = u32_at(40);
        let resource = &raw[312..824];
        let resource = if domain == 5 && u32_at(48) == 0 {
            let (ip, port_offset) = match resource[0] {
                4 => (
                    IpAddr::V4(Ipv4Addr::from(
                        <[u8; 4]>::try_from(&resource[1..5]).unwrap(),
                    )),
                    5,
                ),
                6 => (
                    IpAddr::V6(Ipv6Addr::from(
                        <[u8; 16]>::try_from(&resource[1..17]).unwrap(),
                    )),
                    17,
                ),
                _ => return Err("LSM network audit has invalid address family".into()),
            };
            SocketAddr::new(
                ip,
                u16::from_be_bytes(resource[port_offset..port_offset + 2].try_into().unwrap()),
            )
            .to_string()
        } else {
            string(resource)
        };
        Ok(Self {
            revision: u64_at(0),
            trace_id: u64_at(8),
            observed_ktime_ns: u64_at(16),
            start_boottime_ns: u64_at(24),
            kernel_tgid: u32_at(32),
            observer_tgid: u32_at(36),
            domain,
            decision: u32_at(44),
            error: u32_at(48) as i32,
            signal_result: u32_at(52) as i32,
            rule_id: string(&raw[56..184]),
            owner: string(&raw[184..312]),
            resource,
            retain_audit: u64_at(824) != 0,
        })
    }
}
