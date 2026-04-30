//! Nested virtualization detection + toggle.
//!
//! libvirt expresses nested virt as a CPU feature (`vmx` for Intel,
//! `svm` for AMD) under the domain's `<cpu>` block. Three host CPU
//! modes interact with this differently:
//!
//! - `host-passthrough`: the guest sees the host CPU verbatim, so
//!   nested support is automatic IF the host kernel module
//!   (kvm_intel.nested / kvm_amd.nested) is enabled. We don't need
//!   to touch the domain XML — only the host module.
//! - `host-model`: libvirt copies the host CPU description but strips
//!   features by default. We have to explicitly add a
//!   `<feature policy='require' name='vmx|svm'/>` element.
//! - `custom`: same as host-model — explicit feature add required.
//!
//! Source-of-truth for "is nested enabled":
//! 1. Domain XML has the relevant feature (or mode is host-passthrough)
//! 2. Host kernel module's `nested` parameter is "Y" or "1"

use serde::{Deserialize, Serialize};

/// Vendor extracted from the host capabilities XML.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

impl CpuVendor {
    pub fn nested_feature(&self) -> Option<&'static str> {
        match self {
            CpuVendor::Intel => Some("vmx"),
            CpuVendor::Amd => Some("svm"),
            CpuVendor::Unknown => None,
        }
    }

    pub fn nested_module_path(&self) -> Option<&'static str> {
        match self {
            CpuVendor::Intel => Some("/sys/module/kvm_intel/parameters/nested"),
            CpuVendor::Amd => Some("/sys/module/kvm_amd/parameters/nested"),
            CpuVendor::Unknown => None,
        }
    }
}

/// Parse host vendor from the libvirt capabilities XML.
pub fn parse_host_vendor(caps_xml: &str) -> CpuVendor {
    // <host><cpu><vendor>GenuineIntel</vendor>...
    if let Some(start) = caps_xml.find("<vendor>") {
        let after = &caps_xml[start + "<vendor>".len()..];
        if let Some(end) = after.find("</vendor>") {
            let v = after[..end].trim();
            return match v {
                "GenuineIntel" => CpuVendor::Intel,
                "AuthenticAMD" => CpuVendor::Amd,
                _ => CpuVendor::Unknown,
            };
        }
    }
    CpuVendor::Unknown
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NestedVirtState {
    pub vendor: CpuVendor,
    /// CPU mode of the domain ("host-passthrough" | "host-model" | "custom").
    pub cpu_mode: String,
    /// Whether the domain currently has the appropriate vmx/svm feature
    /// set to require/force, OR the mode is host-passthrough (which
    /// implies inheritance).
    pub enabled_in_domain: bool,
    /// Whether the host kernel module reports nested=Y. None when we
    /// couldn't read the sysfs path (different vendor, no SSH).
    pub enabled_in_host: Option<bool>,
}

/// Determine `enabled_in_domain` from a parsed CpuConfig.
/// `cpu_mode`: "host-passthrough" | "host-model" | "custom" | "" (none).
/// `features`: list of (name, policy) pairs from `<feature>` elements.
pub fn domain_nested_enabled(
    vendor: CpuVendor,
    cpu_mode: &str,
    features: &[(String, String)],
) -> bool {
    if cpu_mode == "host-passthrough" {
        // Inherits from host. We treat as "enabled" — the user can
        // verify via the host probe.
        return true;
    }
    let Some(needed) = vendor.nested_feature() else { return false };
    features
        .iter()
        .any(|(n, p)| n == needed && (p == "require" || p == "force"))
}

/// Parse `Y` / `1` (enabled) vs anything else from a sysfs nested
/// parameter file.
pub fn parse_nested_module_value(s: &str) -> bool {
    let t = s.trim();
    matches!(t, "Y" | "y" | "1" | "true")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_intel_vendor() {
        let caps = r#"<capabilities>
<host>
  <cpu>
    <arch>x86_64</arch>
    <vendor>GenuineIntel</vendor>
  </cpu>
</host>
</capabilities>"#;
        assert_eq!(parse_host_vendor(caps), CpuVendor::Intel);
    }

    #[test]
    fn extracts_amd_vendor() {
        let caps = "<host><cpu><vendor>AuthenticAMD</vendor></cpu></host>";
        assert_eq!(parse_host_vendor(caps), CpuVendor::Amd);
    }

    #[test]
    fn unknown_vendor_is_explicit() {
        assert_eq!(parse_host_vendor("<host/>"), CpuVendor::Unknown);
    }

    #[test]
    fn host_passthrough_implies_nested() {
        assert!(domain_nested_enabled(CpuVendor::Intel, "host-passthrough", &[]));
        assert!(domain_nested_enabled(CpuVendor::Amd, "host-passthrough", &[]));
    }

    #[test]
    fn host_model_requires_explicit_vmx_or_svm() {
        let intel_with = vec![("vmx".into(), "require".into())];
        assert!(domain_nested_enabled(CpuVendor::Intel, "host-model", &intel_with));

        let amd_with = vec![("svm".into(), "force".into())];
        assert!(domain_nested_enabled(CpuVendor::Amd, "host-model", &amd_with));

        let intel_without: Vec<(String, String)> = vec![];
        assert!(!domain_nested_enabled(CpuVendor::Intel, "host-model", &intel_without));

        // Disable policy doesn't count.
        let intel_disabled = vec![("vmx".into(), "disable".into())];
        assert!(!domain_nested_enabled(CpuVendor::Intel, "host-model", &intel_disabled));
    }

    #[test]
    fn nested_module_value_recognises_truthy_strings() {
        assert!(parse_nested_module_value("Y"));
        assert!(parse_nested_module_value("y\n"));
        assert!(parse_nested_module_value("1"));
        assert!(parse_nested_module_value("true"));
        assert!(!parse_nested_module_value("N"));
        assert!(!parse_nested_module_value("0"));
        assert!(!parse_nested_module_value(""));
    }

    #[test]
    fn vendor_to_feature_and_module_path() {
        assert_eq!(CpuVendor::Intel.nested_feature(), Some("vmx"));
        assert_eq!(CpuVendor::Amd.nested_feature(), Some("svm"));
        assert_eq!(CpuVendor::Unknown.nested_feature(), None);
        assert!(CpuVendor::Intel.nested_module_path().unwrap().contains("kvm_intel"));
        assert!(CpuVendor::Amd.nested_module_path().unwrap().contains("kvm_amd"));
    }
}
