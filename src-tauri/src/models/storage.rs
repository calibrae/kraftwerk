use serde::{Deserialize, Serialize};

/// Summary info for a storage pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoragePoolInfo {
    pub name: String,
    pub uuid: String,
    pub pool_type: String, // "dir", "logical", "netfs", "iscsi", etc.
    pub is_active: bool,
    pub is_persistent: bool,
    pub autostart: bool,
    /// Capacity in bytes.
    pub capacity: u64,
    pub allocation: u64,
    pub available: u64,
    /// Filesystem path where volumes live (for "dir" pools).
    pub target_path: Option<String>,
    pub num_volumes: u32,
}

/// Summary info for a storage volume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageVolumeInfo {
    pub name: String,
    pub path: String,
    pub key: String,
    pub capacity: u64,
    pub allocation: u64,
    pub format: String, // "qcow2", "raw", "iso", etc.
    pub pool_name: String,
}

/// Supported pool types for the creation wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PoolType {
    Dir,
    Netfs,
    Logical,
    Iscsi,
}

impl PoolType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dir => "dir",
            Self::Netfs => "netfs",
            Self::Logical => "logical",
            Self::Iscsi => "iscsi",
        }
    }
}

/// Volume formats supported by the creation wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeFormat {
    Qcow2,
    Raw,
    Iso,
}

impl VolumeFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Qcow2 => "qcow2",
            Self::Raw => "raw",
            Self::Iso => "iso",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_type_serializes_lowercase() {
        let json = serde_json::to_string(&PoolType::Dir).unwrap();
        assert_eq!(json, "\"dir\"");
        let json = serde_json::to_string(&PoolType::Netfs).unwrap();
        assert_eq!(json, "\"netfs\"");
    }

    #[test]
    fn pool_type_as_str() {
        assert_eq!(PoolType::Dir.as_str(), "dir");
        assert_eq!(PoolType::Logical.as_str(), "logical");
    }

    #[test]
    fn volume_format_as_str() {
        assert_eq!(VolumeFormat::Qcow2.as_str(), "qcow2");
        assert_eq!(VolumeFormat::Raw.as_str(), "raw");
        assert_eq!(VolumeFormat::Iso.as_str(), "iso");
    }

    #[test]
    fn pool_info_serializes() {
        let p = StoragePoolInfo {
            name: "default".into(),
            uuid: "u".into(),
            pool_type: "dir".into(),
            is_active: true,
            is_persistent: true,
            autostart: true,
            capacity: 1_000_000_000,
            allocation: 500_000_000,
            available: 500_000_000,
            target_path: Some("/var/lib/libvirt/images".into()),
            num_volumes: 3,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"pool_type\":\"dir\""));
        assert!(json.contains("\"is_active\":true"));
        assert!(json.contains("\"num_volumes\":3"));
    }

    #[test]
    fn volume_info_serializes() {
        let v = StorageVolumeInfo {
            name: "disk.qcow2".into(),
            path: "/var/lib/libvirt/images/disk.qcow2".into(),
            key: "/var/lib/libvirt/images/disk.qcow2".into(),
            capacity: 10_737_418_240,
            allocation: 6_257_197_056,
            format: "qcow2".into(),
            pool_name: "default".into(),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("\"format\":\"qcow2\""));
        assert!(json.contains("\"pool_name\":\"default\""));
    }
}
