//! Numeric network prefixes and inclusive port scopes shared by grants and rules.
use std::fmt::{Display, Formatter};
use std::net::{IpAddr, SocketAddr};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NetworkPolicyEndpointSelector {
    network: IpAddr,
    prefix: u8,
    first_port: u16,
    last_port: u16,
    scope_id: u32,
}

impl NetworkPolicyEndpointSelector {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if let Ok(endpoint) = raw.parse::<SocketAddr>() {
            return Ok(Self::from_endpoint(endpoint));
        }
        let (address, ports) = raw
            .rsplit_once(':')
            .ok_or_else(|| format!("network selector {raw} requires an address and port scope"))?;
        let bracketed = address.starts_with('[') && address.ends_with(']');
        let address = if bracketed {
            &address[1..address.len() - 1]
        } else {
            address
        };
        let (address, prefix) = match address.split_once('/') {
            Some((address, prefix)) => (
                address,
                Some(
                    prefix
                        .parse::<u8>()
                        .map_err(|_| format!("invalid network prefix in {raw}"))?,
                ),
            ),
            None => (address, None),
        };
        let ip = address
            .parse::<IpAddr>()
            .map_err(|_| format!("network selector {raw} requires a numeric IP"))?;
        if ip.is_ipv6() != bracketed {
            return Err(
                "IPv6 selectors require [address/prefix]:ports; IPv4 selectors have no brackets"
                    .into(),
            );
        }
        let bits = if ip.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(bits);
        if prefix > bits {
            return Err(format!("network prefix exceeds {bits} bits"));
        }
        let (first_port, last_port) = if ports == "*" {
            (0, u16::MAX)
        } else {
            let (first, last) = ports.split_once('-').unwrap_or((ports, ports));
            let first = first.parse::<u16>().map_err(|_| "invalid first port")?;
            let last = last.parse::<u16>().map_err(|_| "invalid last port")?;
            if first > last {
                return Err("port range must be ascending".into());
            }
            (first, last)
        };
        Ok(Self {
            network: Self::network_at(ip, prefix),
            prefix,
            first_port,
            last_port,
            scope_id: 0,
        })
    }

    pub fn from_endpoint(endpoint: SocketAddr) -> Self {
        Self {
            network: endpoint.ip(),
            prefix: if endpoint.is_ipv4() { 32 } else { 128 },
            first_port: endpoint.port(),
            last_port: endpoint.port(),
            scope_id: match endpoint {
                SocketAddr::V6(v6) => v6.scope_id(),
                _ => 0,
            },
        }
    }

    pub fn network(self) -> IpAddr {
        self.network
    }
    pub fn prefix(self) -> u8 {
        self.prefix
    }
    pub fn ports(self) -> (u16, u16) {
        (self.first_port, self.last_port)
    }
    pub fn scope_id(self) -> u32 {
        self.scope_id
    }

    pub fn network_at(ip: IpAddr, prefix: u8) -> IpAddr {
        match ip {
            IpAddr::V4(ip) => IpAddr::V4(
                (u32::from(ip) & u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0)).into(),
            ),
            IpAddr::V6(ip) => IpAddr::V6(
                (u128::from(ip) & u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0))
                    .into(),
            ),
        }
    }

    pub fn covers(self, other: Self) -> bool {
        self.network.is_ipv4() == other.network.is_ipv4()
            && (self.scope_id == 0 || self.scope_id == other.scope_id)
            && self.prefix <= other.prefix
            && Self::network_at(other.network, self.prefix) == self.network
            && self.first_port <= other.first_port
            && self.last_port >= other.last_port
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.network.is_ipv4() == other.network.is_ipv4()
            && (self.scope_id == 0 || other.scope_id == 0 || self.scope_id == other.scope_id)
            && Self::network_at(self.network, self.prefix.min(other.prefix))
                == Self::network_at(other.network, self.prefix.min(other.prefix))
            && self.first_port <= other.last_port
            && other.first_port <= self.last_port
    }
}

impl Display for NetworkPolicyEndpointSelector {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.network.is_ipv6() {
            write!(f, "[")?;
        }
        write!(f, "{}", self.network)?;
        if self.scope_id != 0 {
            write!(f, "%{}", self.scope_id)?;
        }
        if self.prefix != if self.network.is_ipv4() { 32 } else { 128 } {
            write!(f, "/{}", self.prefix)?;
        }
        if self.network.is_ipv6() {
            write!(f, "]")?;
        }
        write!(f, ":")?;
        match (self.first_port, self.last_port) {
            (0, u16::MAX) => write!(f, "*"),
            (first, last) if first == last => write!(f, "{first}"),
            (first, last) => write!(f, "{first}-{last}"),
        }
    }
}
