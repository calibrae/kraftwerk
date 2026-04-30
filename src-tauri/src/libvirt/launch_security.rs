//! Confidential-compute launch security: AMD SEV / SEV-ES / SEV-SNP and
//! Intel TDX.
//!
//! The domain XML expresses this as a `<launchSecurity type='...'>`
//! element directly under `<domain>`. Common shape (SEV):
//!
//! ```xml
//! <launchSecurity type='sev' kernelHashes='no'>
//!   <cbitpos>47</cbitpos>
//!   <reducedPhysBits>1</reducedPhysBits>
//!   <policy>0x0001</policy>
//!   <session>BASE64...</session>
//!   <dhCert>BASE64...</dhCert>
//! </launchSecurity>
//! ```
//!
//! `cbitpos` and `reducedPhysBits` come from the host's domain
//! capabilities — populating them wrong yields a guest that won't
//! boot, so we always source them from `domain_caps::FeatureCaps`
//! when enabling.
//!
//! This module is intentionally read-mostly. Writing SEV requires
//! the operator to have a deployment chain-of-trust (PDH cert + DH
//! session + launch blob from `sevtool`); we don't generate those.
//! What we do support:
//!
//! - parse the current `<launchSecurity>` block (any type)
//! - serialise a SEV/SEV-ES config to XML
//! - apply / remove the block in a domain XML in place
//!
//! SEV-SNP write and TDX write are intentionally not implemented —
//! their parameter sets (idBlock, idAuth, hostData, mrConfigId, ...)
//! need an operator-managed key bundle that doesn't belong in a UI
//! input box. We display them when present and let the operator
//! shape them via XML edit.

use serde::{Deserialize, Serialize};

use crate::models::error::VirtManagerError;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchSecurityKind {
    Sev,
    SevSnp,
    Tdx,
    Other,
}

impl LaunchSecurityKind {
    pub fn from_attr(s: &str) -> Self {
        match s {
            "sev" => Self::Sev,
            "sev-snp" => Self::SevSnp,
            "tdx" => Self::Tdx,
            _ => Self::Other,
        }
    }

    pub fn as_attr(&self) -> &'static str {
        match self {
            Self::Sev => "sev",
            Self::SevSnp => "sev-snp",
            Self::Tdx => "tdx",
            Self::Other => "other",
        }
    }
}

/// Default SEV policy for a sensible "private guest" baseline:
/// bit 0 = NODBG (no debug)
/// bit 1 = NOKS (no key sharing)
/// bit 2 = ES (SEV-ES)
/// → 0x0007. We default to 0x0003 (no ES) for broader compatibility.
pub const DEFAULT_SEV_POLICY: u32 = 0x0003;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LaunchSecurityConfig {
    pub kind: Option<LaunchSecurityKindWrap>,
    /// Hex policy value, formatted as the XML stores it (e.g. "0x0003").
    pub policy: Option<String>,
    pub cbitpos: Option<u32>,
    pub reduced_phys_bits: Option<u32>,
    /// `<session>` body — usually base64. Display-only here.
    pub session: Option<String>,
    /// `<dhCert>` body — usually base64.
    pub dh_cert: Option<String>,
    /// `kernelHashes='yes'` attribute (SEV-only sanity field).
    pub kernel_hashes: bool,
}

/// Newtype around `LaunchSecurityKind` so the default-derived
/// `LaunchSecurityConfig` can omit it (`None` → no element at all).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchSecurityKindWrap(pub LaunchSecurityKind);

impl LaunchSecurityKindWrap {
    pub fn kind(&self) -> LaunchSecurityKind { self.0 }
}

/// Parse `<launchSecurity>` from a domain XML. Returns Ok(None) when
/// absent.
pub fn parse_launch_security(xml: &str) -> Result<Option<LaunchSecurityConfig>, VirtManagerError> {
    let Some(start) = xml.find("<launchSecurity") else { return Ok(None); };
    // Slice from the element start to its closing tag (or self-close).
    let rest = &xml[start..];
    let block_end = if let Some(self_close) = rest.find("/>") {
        // Self-closing only valid when no children: <launchSecurity type='x'/>.
        // Confirm there's no </launchSecurity> before this.
        let close_idx = rest.find("</launchSecurity>");
        match close_idx {
            Some(c) if c < self_close => c + "</launchSecurity>".len(),
            _ => self_close + 2,
        }
    } else if let Some(c) = rest.find("</launchSecurity>") {
        c + "</launchSecurity>".len()
    } else {
        return Err(VirtManagerError::XmlParsingFailed {
            reason: "<launchSecurity> not closed".into(),
        });
    };
    let block = &rest[..block_end];

    let kind = extract_attr(block, "type")
        .map(|s| LaunchSecurityKindWrap(LaunchSecurityKind::from_attr(&s)));
    let kernel_hashes = extract_attr(block, "kernelHashes")
        .as_deref() == Some("yes");

    let policy = extract_text(block, "policy");
    let cbitpos = extract_text(block, "cbitpos").and_then(|s| s.trim().parse().ok());
    let reduced_phys_bits = extract_text(block, "reducedPhysBits")
        .and_then(|s| s.trim().parse().ok());
    let session = extract_text(block, "session");
    let dh_cert = extract_text(block, "dhCert");

    Ok(Some(LaunchSecurityConfig {
        kind,
        policy,
        cbitpos,
        reduced_phys_bits,
        session,
        dh_cert,
        kernel_hashes,
    }))
}

fn extract_attr(block: &str, key: &str) -> Option<String> {
    // Find `key='...'` or `key="..."` inside the opening tag only.
    let head_end = block.find('>').unwrap_or(block.len());
    let head = &block[..head_end];
    for q in ['\'', '"'] {
        let needle = format!("{key}={q}");
        if let Some(s) = head.find(&needle) {
            let after = &head[s + needle.len()..];
            if let Some(e) = after.find(q) {
                return Some(after[..e].to_string());
            }
        }
    }
    None
}

fn extract_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = block.find(&open)? + open.len();
    let e = block[s..].find(&close)?;
    Some(block[s..s + e].to_string())
}

/// Serialise a SEV (or SEV-ES) config to a `<launchSecurity>` block.
/// SEV-SNP and TDX are not built here — they require operator-managed
/// blobs. Caller must validate cbitpos / reducedPhysBits are populated
/// for SEV.
pub fn build_sev_xml(cfg: &LaunchSecurityConfig) -> Result<String, VirtManagerError> {
    let kind = cfg.kind.map(|w| w.0).unwrap_or(LaunchSecurityKind::Sev);
    if !matches!(kind, LaunchSecurityKind::Sev) {
        return Err(VirtManagerError::OperationFailed {
            operation: "buildLaunchSecurity".into(),
            reason: format!("only SEV write is supported here; got {kind:?}"),
        });
    }
    let cbit = cfg.cbitpos.ok_or_else(|| VirtManagerError::OperationFailed {
        operation: "buildLaunchSecurity".into(),
        reason: "cbitpos required (read it from host capabilities)".into(),
    })?;
    let rpb = cfg.reduced_phys_bits.ok_or_else(|| VirtManagerError::OperationFailed {
        operation: "buildLaunchSecurity".into(),
        reason: "reducedPhysBits required (read it from host capabilities)".into(),
    })?;
    let policy = cfg.policy.clone()
        .unwrap_or_else(|| format!("0x{:04X}", DEFAULT_SEV_POLICY));
    let kh = if cfg.kernel_hashes { " kernelHashes='yes'" } else { "" };

    let mut s = String::new();
    s.push_str(&format!("<launchSecurity type='sev'{kh}>\n"));
    s.push_str(&format!("    <cbitpos>{cbit}</cbitpos>\n"));
    s.push_str(&format!("    <reducedPhysBits>{rpb}</reducedPhysBits>\n"));
    s.push_str(&format!("    <policy>{policy}</policy>\n"));
    if let Some(sess) = &cfg.session { s.push_str(&format!("    <session>{sess}</session>\n")); }
    if let Some(dh)  = &cfg.dh_cert  { s.push_str(&format!("    <dhCert>{dh}</dhCert>\n")); }
    s.push_str("  </launchSecurity>");
    Ok(s)
}

/// Splice (or remove) the `<launchSecurity>` block in a domain XML.
/// `cfg = None` removes; `cfg = Some(...)` inserts/replaces.
pub fn apply_launch_security(xml: &str, cfg: Option<&LaunchSecurityConfig>) -> Result<String, VirtManagerError> {
    let new_block = match cfg {
        Some(c) => Some(build_sev_xml(c)?),
        None => None,
    };

    // Locate any existing block.
    if let Some(start) = xml.find("<launchSecurity") {
        let rest = &xml[start..];
        let block_end = if let Some(close_rel) = rest.find("</launchSecurity>") {
            close_rel + "</launchSecurity>".len()
        } else if let Some(self_close) = rest.find("/>") {
            self_close + 2
        } else {
            return Err(VirtManagerError::XmlParsingFailed {
                reason: "<launchSecurity> not closed".into(),
            });
        };
        let absolute_end = start + block_end;
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..start]);
        if let Some(b) = new_block {
            out.push_str(&b);
        } else {
            // Drop the block + any one trailing newline.
            let mut tail = &xml[absolute_end..];
            if tail.starts_with('\n') { tail = &tail[1..]; }
            out.push_str(tail);
            return Ok(out);
        }
        out.push_str(&xml[absolute_end..]);
        return Ok(out);
    }

    // No existing block — only meaningful when adding.
    let Some(b) = new_block else { return Ok(xml.to_string()); };

    // Insert before `</domain>`. Indent two spaces, newline before close.
    let close = "</domain>";
    let Some(pos) = xml.rfind(close) else {
        return Err(VirtManagerError::XmlParsingFailed {
            reason: "domain XML has no </domain>".into(),
        });
    };
    let mut out = String::with_capacity(xml.len() + b.len() + 4);
    out.push_str(&xml[..pos]);
    out.push_str("  ");
    out.push_str(&b);
    out.push('\n');
    out.push_str(&xml[pos..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sev_block() {
        let xml = r#"<domain>
  <launchSecurity type='sev' kernelHashes='no'>
    <cbitpos>47</cbitpos>
    <reducedPhysBits>1</reducedPhysBits>
    <policy>0x0003</policy>
  </launchSecurity>
</domain>"#;
        let cfg = parse_launch_security(xml).unwrap().unwrap();
        assert_eq!(cfg.kind.unwrap().0, LaunchSecurityKind::Sev);
        assert_eq!(cfg.cbitpos, Some(47));
        assert_eq!(cfg.reduced_phys_bits, Some(1));
        assert_eq!(cfg.policy.as_deref(), Some("0x0003"));
        assert!(!cfg.kernel_hashes);
    }

    #[test]
    fn parses_sev_snp_kind() {
        let xml = "<domain><launchSecurity type='sev-snp'><policy>0x30000</policy></launchSecurity></domain>";
        let cfg = parse_launch_security(xml).unwrap().unwrap();
        assert_eq!(cfg.kind.unwrap().0, LaunchSecurityKind::SevSnp);
    }

    #[test]
    fn no_launch_security_returns_none() {
        let cfg = parse_launch_security("<domain/>").unwrap();
        assert!(cfg.is_none());
    }

    #[test]
    fn build_sev_requires_cbitpos() {
        let cfg = LaunchSecurityConfig {
            kind: Some(LaunchSecurityKindWrap(LaunchSecurityKind::Sev)),
            ..Default::default()
        };
        assert!(build_sev_xml(&cfg).is_err());

        let cfg2 = LaunchSecurityConfig {
            kind: Some(LaunchSecurityKindWrap(LaunchSecurityKind::Sev)),
            cbitpos: Some(47),
            reduced_phys_bits: Some(1),
            ..Default::default()
        };
        let s = build_sev_xml(&cfg2).unwrap();
        assert!(s.contains("type='sev'"));
        assert!(s.contains("<cbitpos>47</cbitpos>"));
        assert!(s.contains("0x0003")); // default policy
    }

    #[test]
    fn build_rejects_snp_and_tdx() {
        let cfg = LaunchSecurityConfig {
            kind: Some(LaunchSecurityKindWrap(LaunchSecurityKind::SevSnp)),
            cbitpos: Some(47),
            reduced_phys_bits: Some(1),
            ..Default::default()
        };
        assert!(build_sev_xml(&cfg).is_err());
    }

    #[test]
    fn apply_inserts_when_absent() {
        let xml = "<domain>\n  <name>vm</name>\n</domain>";
        let cfg = LaunchSecurityConfig {
            kind: Some(LaunchSecurityKindWrap(LaunchSecurityKind::Sev)),
            cbitpos: Some(47),
            reduced_phys_bits: Some(1),
            ..Default::default()
        };
        let out = apply_launch_security(xml, Some(&cfg)).unwrap();
        assert!(out.contains("<launchSecurity type='sev'"));
        assert!(out.contains("</launchSecurity>"));
        assert!(out.find("<launchSecurity").unwrap() < out.find("</domain>").unwrap());
    }

    #[test]
    fn apply_replaces_existing() {
        let xml = "<domain>\n  <launchSecurity type='sev'>\n    <cbitpos>40</cbitpos>\n    <reducedPhysBits>1</reducedPhysBits>\n    <policy>0x0001</policy>\n  </launchSecurity>\n</domain>";
        let cfg = LaunchSecurityConfig {
            kind: Some(LaunchSecurityKindWrap(LaunchSecurityKind::Sev)),
            cbitpos: Some(47),
            reduced_phys_bits: Some(1),
            policy: Some("0x0007".into()),
            ..Default::default()
        };
        let out = apply_launch_security(xml, Some(&cfg)).unwrap();
        assert!(out.contains("<cbitpos>47</cbitpos>"));
        assert!(out.contains("<policy>0x0007</policy>"));
        assert!(!out.contains("<cbitpos>40</cbitpos>"));
    }

    #[test]
    fn apply_removes_when_none() {
        let xml = "<domain>\n  <launchSecurity type='sev'>\n    <cbitpos>47</cbitpos>\n    <reducedPhysBits>1</reducedPhysBits>\n    <policy>0x0003</policy>\n  </launchSecurity>\n</domain>";
        let out = apply_launch_security(xml, None).unwrap();
        assert!(!out.contains("launchSecurity"));
        assert!(out.contains("</domain>"));
    }
}
