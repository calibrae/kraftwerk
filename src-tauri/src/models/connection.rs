use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Authentication method for connecting to a hypervisor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    SshKey,
    Password,
    SshAgent,
}

/// A saved hypervisor connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    pub id: Uuid,
    pub display_name: String,
    pub uri: String,
    pub auth_type: AuthType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_connected: Option<i64>,
}

impl SavedConnection {
    pub fn new(display_name: String, uri: String, auth_type: AuthType) -> Self {
        Self {
            id: Uuid::new_v4(),
            display_name,
            uri,
            auth_type,
            last_connected: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_connection_has_unique_id() {
        let a = SavedConnection::new("a".into(), "qemu+ssh://h/system".into(), AuthType::SshAgent);
        let b = SavedConnection::new("b".into(), "qemu+ssh://h/system".into(), AuthType::SshAgent);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn serializes_auth_type_as_snake_case() {
        let conn = SavedConnection::new("test".into(), "qemu:///system".into(), AuthType::SshKey);
        let json = serde_json::to_string(&conn).unwrap();
        assert!(json.contains("\"ssh_key\""));
    }

    #[test]
    fn round_trip_serialization() {
        let conn = SavedConnection::new("dev".into(), "qemu+ssh://h/system".into(), AuthType::Password);
        let json = serde_json::to_string(&conn).unwrap();
        let deserialized: SavedConnection = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.display_name, "dev");
        assert_eq!(deserialized.uri, "qemu+ssh://h/system");
        assert_eq!(deserialized.auth_type, AuthType::Password);
    }
}
