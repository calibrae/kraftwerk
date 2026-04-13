use serde::{Deserialize, Serialize};

/// Summary info for a virtual network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub name: String,
    pub uuid: String,
    pub is_active: bool,
    pub is_persistent: bool,
    pub autostart: bool,
    pub bridge: Option<String>,
    pub forward_mode: String,
    pub ipv4_summary: Option<String>,
    pub ipv6_summary: Option<String>,
}

/// Forward modes for libvirt networks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ForwardMode {
    Nat,
    Route,
    Open,
    Bridge,
    #[default]
    Isolated,
    Private,
    Vepa,
    Passthrough,
    Hostdev,
}

impl ForwardMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "nat" => Self::Nat,
            "route" | "routed" => Self::Route,
            "open" => Self::Open,
            "bridge" => Self::Bridge,
            "private" => Self::Private,
            "vepa" => Self::Vepa,
            "passthrough" => Self::Passthrough,
            "hostdev" => Self::Hostdev,
            _ => Self::Isolated,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nat => "nat",
            Self::Route => "route",
            Self::Open => "open",
            Self::Bridge => "bridge",
            Self::Isolated => "isolated",
            Self::Private => "private",
            Self::Vepa => "vepa",
            Self::Passthrough => "passthrough",
            Self::Hostdev => "hostdev",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_mode_round_trip() {
        for mode in [
            ForwardMode::Nat,
            ForwardMode::Route,
            ForwardMode::Open,
            ForwardMode::Bridge,
            ForwardMode::Isolated,
        ] {
            let s = mode.as_str();
            assert_eq!(ForwardMode::from_str(s), mode);
        }
    }

    #[test]
    fn forward_mode_routed_alias() {
        assert_eq!(ForwardMode::from_str("routed"), ForwardMode::Route);
    }

    #[test]
    fn forward_mode_unknown_defaults_to_isolated() {
        assert_eq!(ForwardMode::from_str("wibble"), ForwardMode::Isolated);
    }

    #[test]
    fn forward_mode_case_insensitive() {
        assert_eq!(ForwardMode::from_str("NAT"), ForwardMode::Nat);
        assert_eq!(ForwardMode::from_str("Bridge"), ForwardMode::Bridge);
    }

    #[test]
    fn network_info_serializes() {
        let n = NetworkInfo {
            name: "test".into(),
            uuid: "u".into(),
            is_active: true,
            is_persistent: true,
            autostart: false,
            bridge: Some("virbr0".into()),
            forward_mode: "nat".into(),
            ipv4_summary: Some("192.168.122.1/24".into()),
            ipv6_summary: None,
        };
        let json = serde_json::to_string(&n).unwrap();
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"is_active\":true"));
    }
}
