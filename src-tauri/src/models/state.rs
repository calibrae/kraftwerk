use serde::Serialize;

/// Connection state for a hypervisor connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "message")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_tag() {
        let json = serde_json::to_string(&ConnectionState::Connected).unwrap();
        assert!(json.contains("\"connected\""));

        let json = serde_json::to_string(&ConnectionState::Error("timeout".into())).unwrap();
        assert!(json.contains("\"error\""));
        assert!(json.contains("timeout"));
    }
}
