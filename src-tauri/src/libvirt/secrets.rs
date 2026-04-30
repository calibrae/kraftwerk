//! virSecret CRUD + helpers for LUKS volume encryption.
//!
//! Secrets are libvirt's way of storing sensitive material (volume
//! passphrases, CHAP passwords, Ceph keys) outside of the domain XML
//! the user manipulates. They're addressed by either UUID or by a
//! `(usage_type, usage_id)` pair.
//!
//! For LUKS-encrypted volumes the canonical pattern is:
//! - usage_type = "volume", usage_id = absolute path to the .qcow2/.raw
//! - secret value = the LUKS passphrase bytes (no terminator, no \n)
//! - the volume XML references the secret via
//!   `<encryption format='luks'><secret type='passphrase' uuid='...'/>`
//!
//! Exposed shapes here:
//! - SecretInfo for the UI list
//! - build_secret_xml / build_luks_volume_xml builders
//! - parse_secret_xml round-tripping for tests

use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;

/// Mirrors libvirt's VIR_SECRET_USAGE_TYPE_* constants. Only the values
/// we actively support are named; arbitrary u32s round-trip through.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecretUsage {
    None,
    Volume,
    Ceph,
    Iscsi,
    Tls,
    Vtpm,
    Unknown(u32),
}

impl SecretUsage {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Volume,
            2 => Self::Ceph,
            3 => Self::Iscsi,
            4 => Self::Tls,
            5 => Self::Vtpm,
            n => Self::Unknown(n),
        }
    }

    pub fn as_xml_attr(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Volume => "volume",
            Self::Ceph => "ceph",
            Self::Iscsi => "iscsi",
            Self::Tls => "tls",
            Self::Vtpm => "vtpm",
            Self::Unknown(_) => "none",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretInfo {
    pub uuid: String,
    pub usage: SecretUsage,
    /// Volume path / Ceph user / iSCSI target. None for type=none.
    pub usage_id: Option<String>,
    pub description: Option<String>,
    /// Whether the secret has a value set. We don't expose the value
    /// itself; the user's UI just shows present/absent.
    pub has_value: bool,
    /// Whether the secret survives libvirtd restart (`<secret ephemeral='no'>`).
    pub ephemeral: bool,
    /// Whether libvirt allows reading the value back via API
    /// (`<secret private='no'>` permits get_value).
    pub private: bool,
}

/// Build the `<secret>` XML used by virSecretDefineXML. `usage_id` is
/// required for type=volume / ceph / iscsi (libvirt rejects without it);
/// for type=none we omit the usage element entirely.
pub fn build_secret_xml(
    usage: SecretUsage,
    usage_id: Option<&str>,
    description: Option<&str>,
    ephemeral: bool,
    private: bool,
) -> String {
    let eph = if ephemeral { "yes" } else { "no" };
    let priv_ = if private { "yes" } else { "no" };
    let mut xml = format!(
        "<secret ephemeral='{eph}' private='{priv_}'>\n"
    );
    if let Some(d) = description {
        if !d.is_empty() {
            xml.push_str(&format!("  <description>{}</description>\n", escape_xml(d)));
        }
    }
    let kind = usage.as_xml_attr();
    if matches!(usage, SecretUsage::None) {
        xml.push_str("  <usage type='none'/>\n");
    } else {
        xml.push_str(&format!("  <usage type='{kind}'>\n"));
        if let Some(id) = usage_id {
            // The inner element name varies by usage type.
            let inner = match usage {
                SecretUsage::Volume => "volume",
                SecretUsage::Ceph => "name",
                SecretUsage::Iscsi => "target",
                SecretUsage::Tls => "name",
                SecretUsage::Vtpm => "name",
                _ => "name",
            };
            xml.push_str(&format!("    <{inner}>{}</{inner}>\n", escape_xml(id)));
        }
        xml.push_str("  </usage>\n");
    }
    xml.push_str("</secret>\n");
    xml
}

/// Parse a libvirt secret XML back into SecretInfo. Used by the
/// list-secrets path (which fetches each secret's XML) and by tests.
/// Caller fills in `has_value` from a separate has-value RPC.
pub fn parse_secret_xml(xml: &str) -> Option<SecretInfo> {
    let uuid = extract_tag(xml, "uuid")?;
    let description = extract_tag(xml, "description");
    let ephemeral = match extract_attr(xml, "<secret ", "ephemeral") {
        Some(s) => s == "yes",
        None => false,
    };
    let private = match extract_attr(xml, "<secret ", "private") {
        Some(s) => s == "yes",
        None => false,
    };
    let kind_attr = extract_attr(xml, "<usage ", "type").unwrap_or_else(|| "none".into());
    let usage = match kind_attr.as_str() {
        "none" => SecretUsage::None,
        "volume" => SecretUsage::Volume,
        "ceph" => SecretUsage::Ceph,
        "iscsi" => SecretUsage::Iscsi,
        "tls" => SecretUsage::Tls,
        "vtpm" => SecretUsage::Vtpm,
        _ => SecretUsage::Unknown(0),
    };
    let usage_id = match usage {
        SecretUsage::Volume => extract_tag(xml, "volume"),
        SecretUsage::Ceph | SecretUsage::Tls | SecretUsage::Vtpm => extract_tag(xml, "name"),
        SecretUsage::Iscsi => extract_tag(xml, "target"),
        _ => None,
    };

    Some(SecretInfo {
        uuid,
        usage,
        usage_id,
        description,
        has_value: false,
        ephemeral,
        private,
    })
}

/// Build a LUKS-encrypted volume XML referencing an existing secret by
/// UUID. Used by the volume-create flow when "Encrypt with LUKS" is on.
pub fn build_luks_volume_xml(name: &str, capacity_bytes: u64, secret_uuid: &str) -> String {
    format!(
        "<volume>\n  <name>{}</name>\n  <capacity unit='B'>{}</capacity>\n  <target>\n    <format type='qcow2'/>\n    <encryption format='luks'>\n      <secret type='passphrase' uuid='{}'/>\n    </encryption>\n  </target>\n</volume>",
        escape_xml(name),
        capacity_bytes,
        escape_xml(secret_uuid),
    )
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

fn extract_attr(xml: &str, tag_prefix: &str, attr: &str) -> Option<String> {
    let i = xml.find(tag_prefix)?;
    let rest = &xml[i..];
    let close = rest.find('>')?;
    let header = &rest[..close];
    for q in ['\'', '"'] {
        let needle = format!("{attr}={q}");
        if let Some(s) = header.find(&needle) {
            let after = &header[s + needle.len()..];
            if let Some(e) = after.find(q) {
                return Some(after[..e].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_volume_secret_xml() {
        let xml = build_secret_xml(
            SecretUsage::Volume,
            Some("/var/lib/libvirt/images/foo.qcow2"),
            Some("LUKS for foo"),
            false,
            true,
        );
        assert!(xml.contains("ephemeral='no'"));
        assert!(xml.contains("private='yes'"));
        assert!(xml.contains("<description>LUKS for foo</description>"));
        assert!(xml.contains("<usage type='volume'>"));
        assert!(xml.contains("<volume>/var/lib/libvirt/images/foo.qcow2</volume>"));
    }

    #[test]
    fn build_none_usage_xml_has_no_inner() {
        let xml = build_secret_xml(SecretUsage::None, None, None, true, false);
        assert!(xml.contains("<usage type='none'/>"));
        assert!(!xml.contains("<volume>"));
    }

    #[test]
    fn parse_round_trip() {
        let xml = r#"<secret ephemeral='no' private='yes'>
  <uuid>11111111-2222-3333-4444-555555555555</uuid>
  <description>luks foo</description>
  <usage type='volume'>
    <volume>/var/lib/libvirt/images/foo.qcow2</volume>
  </usage>
</secret>"#;
        let info = parse_secret_xml(xml).unwrap();
        assert_eq!(info.uuid, "11111111-2222-3333-4444-555555555555");
        assert_eq!(info.usage, SecretUsage::Volume);
        assert_eq!(info.usage_id.as_deref(), Some("/var/lib/libvirt/images/foo.qcow2"));
        assert_eq!(info.description.as_deref(), Some("luks foo"));
        assert!(!info.ephemeral);
        assert!(info.private);
    }

    #[test]
    fn build_luks_volume_xml_references_uuid() {
        let xml = build_luks_volume_xml("foo.qcow2", 21_474_836_480, "abc-uuid");
        assert!(xml.contains("<encryption format='luks'>"));
        assert!(xml.contains("<secret type='passphrase' uuid='abc-uuid'/>"));
        assert!(xml.contains("<capacity unit='B'>21474836480</capacity>"));
    }

    #[test]
    fn escape_xml_in_inputs() {
        let xml = build_secret_xml(SecretUsage::Volume, Some("/a&b"), Some("foo<>"), false, false);
        assert!(xml.contains("/a&amp;b"));
        assert!(xml.contains("foo&lt;&gt;"));
    }
}
