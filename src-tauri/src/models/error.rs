use serde::Serialize;
use thiserror::Error;

/// Application-level errors.
#[derive(Debug, Error)]
pub enum VirtManagerError {
    #[error("Connection failed to {host}: {reason}")]
    ConnectionFailed { host: String, reason: String },

    #[error("Authentication failed for {host}")]
    AuthenticationFailed { host: String },

    #[error("Connection to {host} timed out")]
    Timeout { host: String },

    #[error("SSH host key verification failed for {host}")]
    HostKeyVerificationFailed { host: String },

    #[error("Not connected to hypervisor")]
    NotConnected,

    #[error("VM '{name}' not found")]
    DomainNotFound { name: String },

    #[error("{operation} failed: {reason}")]
    OperationFailed { operation: String, reason: String },

    #[error("Failed to parse XML: {reason}")]
    XmlParsingFailed { reason: String },

    #[error("Credential store error: {0}")]
    CredentialStore(String),

    #[error("Connection '{id}' not found")]
    ConnectionNotFound { id: String },
}

/// Serializable error payload for the frontend.
#[derive(Debug, Serialize)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    pub suggestion: Option<String>,
}

impl From<&VirtManagerError> for ErrorPayload {
    fn from(err: &VirtManagerError) -> Self {
        let (code, suggestion) = match err {
            VirtManagerError::ConnectionFailed { .. } => (
                "connection_failed",
                Some("Check the hostname and ensure the libvirt daemon is running."),
            ),
            VirtManagerError::AuthenticationFailed { .. } => (
                "auth_failed",
                Some("Verify your credentials and SSH key configuration."),
            ),
            VirtManagerError::Timeout { .. } => (
                "timeout",
                Some("Check network connectivity and firewall settings."),
            ),
            VirtManagerError::HostKeyVerificationFailed { .. } => (
                "host_key_failed",
                Some("Verify the host key fingerprint or remove the old key."),
            ),
            VirtManagerError::NotConnected => (
                "not_connected",
                Some("Connect to a hypervisor first."),
            ),
            VirtManagerError::DomainNotFound { .. } => (
                "domain_not_found",
                Some("Refresh the VM list."),
            ),
            VirtManagerError::OperationFailed { .. } => (
                "operation_failed",
                Some("Try the operation again."),
            ),
            VirtManagerError::XmlParsingFailed { .. } => (
                "xml_parsing_failed",
                Some("The VM configuration may be corrupted."),
            ),
            VirtManagerError::CredentialStore(_) => (
                "credential_store",
                Some("Check keychain/keyring access permissions."),
            ),
            VirtManagerError::ConnectionNotFound { .. } => (
                "connection_not_found",
                None,
            ),
        };

        ErrorPayload {
            code: code.to_string(),
            message: err.to_string(),
            suggestion: suggestion.map(String::from),
        }
    }
}

// Allow VirtManagerError to be returned from Tauri commands
impl Serialize for VirtManagerError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ErrorPayload::from(self).serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        let err = VirtManagerError::ConnectionFailed {
            host: "server1".into(),
            reason: "refused".into(),
        };
        assert_eq!(err.to_string(), "Connection failed to server1: refused");
    }

    #[test]
    fn error_payload_has_code_and_suggestion() {
        let err = VirtManagerError::NotConnected;
        let payload = ErrorPayload::from(&err);
        assert_eq!(payload.code, "not_connected");
        assert!(payload.suggestion.is_some());
    }

    #[test]
    fn error_serializes_as_payload() {
        let err = VirtManagerError::DomainNotFound { name: "test-vm".into() };
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"domain_not_found\""));
        assert!(json.contains("test-vm"));
    }
}
