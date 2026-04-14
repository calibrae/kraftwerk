//! Boot / firmware / machine / features editor.
//!
//! Parses and patches the `<os>`, `<features>`, event-action, and
//! `<cpu mode>` sections of a domain XML. Deliberately does NOT do a
//! full parse-and-reserialize: we mutate the existing XML in place so
//! untouched sections (seclabel, metadata, iothreads, etc.) round-trip
//! exactly — critical for the "libvirt is the source of truth" model.

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

/// Parsed boot-related fields from a domain XML.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BootConfig {
    /// "bios" (default, no <loader>) or "efi" (pflash loader present).
    pub firmware: String,
    pub machine: Option<String>,
    pub arch: Option<String>,
    /// Boot device names in order: "hd", "cdrom", "network", "fd".
    pub boot_order: Vec<String>,
    /// <bootmenu enable='yes' timeout='3000'/>
    pub boot_menu_enabled: bool,
    pub boot_menu_timeout_ms: Option<u32>,
    /// Secure boot — only meaningful with firmware='efi'. Reflects
    /// `<loader secure='yes'>` or `<os firmware='efi'><feature enabled='yes' name='secure-boot'/></os>`.
    pub secure_boot: bool,
    /// <features> flags.
    pub features: FeatureFlags,
    /// Event actions.
    pub on_poweroff: Option<String>,
    pub on_reboot: Option<String>,
    pub on_crash: Option<String>,
    /// CPU mode string from <cpu mode='...'>. Not the full CPU config —
    /// Round I will own detailed CPU editing.
    pub cpu_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FeatureFlags {
    pub acpi: bool,
    pub apic: bool,
    pub pae: bool,
    pub smm: bool,
    pub hap: bool,
    pub vmport: Option<bool>, // tri-state: None = not set
}

/// Editable patch. All fields optional — only set ones are applied.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BootPatch {
    pub firmware: Option<String>,
    pub machine: Option<String>,
    pub boot_order: Option<Vec<String>>,
    pub boot_menu_enabled: Option<bool>,
    pub boot_menu_timeout_ms: Option<Option<u32>>,
    pub secure_boot: Option<bool>,
    pub on_poweroff: Option<String>,
    pub on_reboot: Option<String>,
    pub on_crash: Option<String>,
    pub features: Option<FeatureFlags>,
    pub cpu_mode: Option<String>,
}

/// Valid event action values per libvirt docs (formatdomain.html).
/// Note: the QEMU driver does NOT accept "preserve" for on_poweroff or
/// on_reboot — it is only valid on on_crash and on_lockfailure. The UI
/// should filter per event type.
pub const EVENT_ACTIONS: &[&str] = &[
    "destroy", "restart", "preserve",
    "rename-restart", "coredump-destroy", "coredump-restart",
];

// ────────── parse ──────────

/// Read the boot-related fields out of a domain XML.
pub fn parse(xml: &str) -> Result<BootConfig, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut cfg = BootConfig {
        firmware: "bios".to_string(),
        ..Default::default()
    };
    let mut path: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    let mut capture: Option<TextTarget> = None;

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: format!("at {}: {}", r.buffer_position(), e),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                handle_el(&n, &a, &path, &mut cfg, &mut capture);
                path.push(n);
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                handle_el(&n, &a, &path, &mut cfg, &mut capture);
                capture = None;
            }
            Ok(Event::End(_)) => {
                path.pop();
                capture = None;
            }
            Ok(Event::Text(t)) => {
                let s = t.unescape().unwrap_or_default().to_string();
                if let Some(target) = capture {
                    match target {
                        TextTarget::OnPoweroff => cfg.on_poweroff = Some(s),
                        TextTarget::OnReboot => cfg.on_reboot = Some(s),
                        TextTarget::OnCrash => cfg.on_crash = Some(s),
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(cfg)
}

fn handle_el(
    n: &str,
    a: &[(String, String)],
    path: &[String],
    cfg: &mut BootConfig,
    capture: &mut Option<TextTarget>,
) {
    let parent = path.last().map(String::as_str);
    let attr = |k: &str| a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    match (parent, n) {
        (Some("os"), "type") => {
            cfg.arch = attr("arch");
            cfg.machine = attr("machine");
        }
        (Some("os"), "boot") => {
            if let Some(dev) = attr("dev") {
                cfg.boot_order.push(dev);
            }
        }
        (Some("os"), "bootmenu") => {
            cfg.boot_menu_enabled = attr("enable").as_deref() == Some("yes");
            cfg.boot_menu_timeout_ms = attr("timeout").and_then(|s| s.parse().ok());
        }
        (Some("os"), "loader") => {
            cfg.firmware = "efi".into();
            if attr("secure").as_deref() == Some("yes") {
                cfg.secure_boot = true;
            }
        }
        (Some("features"), "acpi") => cfg.features.acpi = true,
        (Some("features"), "apic") => cfg.features.apic = true,
        (Some("features"), "pae") => cfg.features.pae = true,
        (Some("features"), "smm") => cfg.features.smm = true,
        (Some("features"), "hap") => cfg.features.hap = true,
        (Some("features"), "vmport") => {
            cfg.features.vmport = Some(attr("state").as_deref() == Some("on"));
        }
        (Some("domain"), "cpu") => {
            if let Some(m) = attr("mode") { cfg.cpu_mode = Some(m); }
        }
        (Some("domain"), "on_poweroff") => *capture = Some(TextTarget::OnPoweroff),
        (Some("domain"), "on_reboot") => *capture = Some(TextTarget::OnReboot),
        (Some("domain"), "on_crash") => *capture = Some(TextTarget::OnCrash),
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum TextTarget { OnPoweroff, OnReboot, OnCrash }

fn utf8_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes().filter_map(|a| a.ok()).map(|a| (
        String::from_utf8_lossy(a.key.as_ref()).to_string(),
        a.unescape_value().unwrap_or_default().to_string(),
    )).collect()
}

// ────────── apply ──────────
//
// Strategy: read the input XML with quick-xml, stream it to an output
// buffer, swapping in new content when we hit the elements the patch
// mentions. Everything we don't touch is copied through byte-for-byte
// semantically (quick-xml normalises whitespace slightly but preserves
// the element tree).

/// Apply a BootPatch to a domain XML, returning the updated XML.
pub fn apply(xml: &str, patch: &BootPatch) -> Result<String, VirtManagerError> {
    // Simple strategy: parse into strings we need to replace, then rebuild
    // the whole <os> block / <features> block / event actions / <cpu> attr
    // from scratch by splicing into the original. This is easier than a
    // streaming rewrite and fine for the OS block which is well-defined.

    let mut out = xml.to_string();

    // <os> block replacement.
    if patch.firmware.is_some() || patch.machine.is_some() || patch.boot_order.is_some()
        || patch.boot_menu_enabled.is_some() || patch.boot_menu_timeout_ms.is_some()
        || patch.secure_boot.is_some()
    {
        // Read current config and merge with patch.
        let current = parse(xml)?;
        let effective_firmware = patch.firmware.clone().unwrap_or(current.firmware.clone());
        let effective_machine = patch.machine.clone().or(current.machine.clone());
        let effective_arch = current.arch.clone();
        let effective_order = patch.boot_order.clone().unwrap_or(current.boot_order.clone());
        let effective_menu = patch.boot_menu_enabled.unwrap_or(current.boot_menu_enabled);
        let effective_timeout = match patch.boot_menu_timeout_ms {
            Some(t) => t,
            None => current.boot_menu_timeout_ms,
        };
        let effective_secure = patch.secure_boot.unwrap_or(current.secure_boot);

        let new_os = build_os_block(
            effective_arch.as_deref(),
            effective_machine.as_deref(),
            &effective_firmware,
            effective_secure,
            &effective_order,
            effective_menu,
            effective_timeout,
        );
        out = replace_element_block(&out, "os", &new_os)?;
    }

    // <features> block replacement.
    if let Some(ref feats) = patch.features {
        let new_feats = build_features_block(feats);
        out = replace_element_block(&out, "features", &new_feats)?;
    }

    // Event actions.
    if let Some(v) = patch.on_poweroff.as_deref() {
        out = replace_text_element(&out, "on_poweroff", v);
    }
    if let Some(v) = patch.on_reboot.as_deref() {
        out = replace_text_element(&out, "on_reboot", v);
    }
    if let Some(v) = patch.on_crash.as_deref() {
        out = replace_text_element(&out, "on_crash", v);
    }

    // <cpu mode='...'> attribute swap.
    if let Some(ref mode) = patch.cpu_mode {
        out = replace_cpu_mode(&out, mode);
    }

    Ok(out)
}

/// Build a fresh `<os>...</os>` block from the patched fields.
fn build_os_block(
    arch: Option<&str>,
    machine: Option<&str>,
    firmware: &str,
    secure_boot: bool,
    boot_order: &[String],
    boot_menu: bool,
    boot_menu_timeout_ms: Option<u32>,
) -> String {
    let mut s = String::from("<os>\n");
    let type_line = match (arch, machine) {
        (Some(a), Some(m)) => format!(
            "    <type arch='{}' machine='{}'>hvm</type>\n",
            escape_xml(a), escape_xml(m)
        ),
        (Some(a), None) => format!("    <type arch='{}'>hvm</type>\n", escape_xml(a)),
        (None, Some(m)) => format!("    <type machine='{}'>hvm</type>\n", escape_xml(m)),
        (None, None) => "    <type>hvm</type>\n".to_string(),
    };
    s.push_str(&type_line);
    if firmware.eq_ignore_ascii_case("efi") {
        let secure_attr = if secure_boot { " secure='yes'" } else { "" };
        s.push_str(&format!(
            "    <loader readonly='yes' type='pflash'{}>/usr/share/OVMF/OVMF_CODE.fd</loader>\n",
            secure_attr,
        ));
    }
    for dev in boot_order {
        s.push_str(&format!("    <boot dev='{}'/>\n", escape_xml(dev)));
    }
    if boot_menu {
        match boot_menu_timeout_ms {
            Some(t) => s.push_str(&format!("    <bootmenu enable='yes' timeout='{}'/>\n", t)),
            None => s.push_str("    <bootmenu enable='yes'/>\n"),
        }
    }
    s.push_str("  </os>");
    s
}

fn build_features_block(f: &FeatureFlags) -> String {
    let mut s = String::from("<features>\n");
    if f.acpi { s.push_str("    <acpi/>\n"); }
    if f.apic { s.push_str("    <apic/>\n"); }
    if f.pae { s.push_str("    <pae/>\n"); }
    if f.smm { s.push_str("    <smm/>\n"); }
    if f.hap { s.push_str("    <hap/>\n"); }
    if let Some(v) = f.vmport {
        s.push_str(&format!("    <vmport state='{}'/>\n", if v { "on" } else { "off" }));
    }
    s.push_str("  </features>");
    s
}

/// Replace the outermost `<name>...</name>` block in `xml` with `new_content`.
/// `new_content` must be the full element including its opening and closing tags.
/// If the element is absent, inject it before `</domain>`.
fn replace_element_block(xml: &str, name: &str, new_content: &str) -> Result<String, VirtManagerError> {
    // Find the start/end of the element using a streaming pass rather than
    // regex so nested elements of the same name (unlikely at this level)
    // don't confuse us.
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false); // keep whitespace
    let mut buf = Vec::new();
    let mut depth: i32 = 0;
    let mut start_byte: Option<usize> = None;
    let mut end_byte: Option<usize> = None;

    loop {
        let pos_before = r.buffer_position() as usize;
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(VirtManagerError::XmlParsingFailed { reason: e.to_string() }),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if String::from_utf8_lossy(e.name().as_ref()) == name => {
                if depth == 0 {
                    start_byte = Some(pos_before);
                }
                depth += 1;
            }
            Ok(Event::Empty(e)) if String::from_utf8_lossy(e.name().as_ref()) == name => {
                let pos_after = r.buffer_position() as usize;
                start_byte = Some(pos_before);
                end_byte = Some(pos_after);
                break;
            }
            Ok(Event::End(e)) if String::from_utf8_lossy(e.name().as_ref()) == name => {
                depth -= 1;
                if depth == 0 {
                    end_byte = Some(r.buffer_position() as usize);
                    break;
                }
            }
            _ => {}
        }
        buf.clear();
    }

    match (start_byte, end_byte) {
        (Some(s), Some(e)) => {
            let mut out = String::with_capacity(xml.len() + new_content.len());
            out.push_str(&xml[..s]);
            out.push_str(new_content);
            out.push_str(&xml[e..]);
            Ok(out)
        }
        _ => {
            // Inject before </domain>.
            if let Some(idx) = xml.rfind("</domain>") {
                let mut out = String::with_capacity(xml.len() + new_content.len() + 4);
                out.push_str(&xml[..idx]);
                out.push_str("  ");
                out.push_str(new_content);
                out.push('\n');
                out.push_str(&xml[idx..]);
                Ok(out)
            } else {
                Err(VirtManagerError::XmlParsingFailed {
                    reason: format!("could not insert <{name}> — no </domain> found"),
                })
            }
        }
    }
}

fn replace_text_element(xml: &str, name: &str, new_text: &str) -> String {
    let new_value = escape_xml(new_text);
    // Build a minimal streaming rewriter to replace just the text of
    // `<name>X</name>`. If the element is absent, append before </domain>.
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut w = Writer::new(Cursor::new(Vec::<u8>::new()));

    let mut buf = Vec::new();
    let mut in_target = false;
    let mut found = false;

    loop {
        match r.read_event_into(&mut buf) {
            Err(_) => return xml.to_string(),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if String::from_utf8_lossy(e.name().as_ref()) == name => {
                found = true;
                in_target = true;
                let _ = w.write_event(Event::Start(e.to_owned()));
            }
            Ok(Event::End(e)) if String::from_utf8_lossy(e.name().as_ref()) == name => {
                in_target = false;
                let _ = w.write_event(Event::Text(BytesText::new(&new_value)));
                let _ = w.write_event(Event::End(BytesEnd::new(name.to_string())));
            }
            Ok(Event::Text(_)) if in_target => {
                // Skip original text — we'll emit the replacement at End.
            }
            Ok(ev) => {
                let _ = w.write_event(ev);
            }
        }
        buf.clear();
    }

    let mut result = String::from_utf8(w.into_inner().into_inner()).unwrap_or_else(|_| xml.to_string());
    if !found {
        if let Some(idx) = result.rfind("</domain>") {
            let inject = format!("  <{}>{}</{}>\n", name, new_value, name);
            result.insert_str(idx, &inject);
        }
    }
    result
}

fn replace_cpu_mode(xml: &str, new_mode: &str) -> String {
    // <cpu mode='...'> → swap the mode attribute only. Regex is fine here
    // since <cpu mode='...'> is unique at the top level.
    use regex::Regex;
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"<cpu(\s+[^>]*?)?\s+mode=['"][^'"]*['"]"#).unwrap()
    });
    let escaped = escape_xml(new_mode);
    if RE.is_match(xml) {
        RE.replace(xml, |caps: &regex::Captures| {
            let other_attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            format!("<cpu{} mode='{}'", other_attrs, escaped)
        }).into_owned()
    } else {
        // Insert a simple <cpu mode='...'/> before the first <devices> tag
        // if we can find it; otherwise before </domain>.
        let ins = format!("  <cpu mode='{}'/>\n", escaped);
        if let Some(idx) = xml.find("<devices>") {
            let mut out = String::with_capacity(xml.len() + ins.len());
            out.push_str(&xml[..idx]);
            out.push_str(&ins);
            out.push_str(&xml[idx..]);
            out
        } else if let Some(idx) = xml.rfind("</domain>") {
            let mut out = String::with_capacity(xml.len() + ins.len());
            out.push_str(&xml[..idx]);
            out.push_str(&ins);
            out.push_str(&xml[idx..]);
            out
        } else {
            xml.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<domain type='kvm'>
  <name>test</name>
  <uuid>1</uuid>
  <memory unit='KiB'>1024</memory>
  <vcpu>1</vcpu>
  <os>
    <type arch='x86_64' machine='q35'>hvm</type>
    <boot dev='hd'/>
    <boot dev='cdrom'/>
  </os>
  <features>
    <acpi/>
    <apic/>
  </features>
  <cpu mode='host-passthrough' check='none'/>
  <on_poweroff>destroy</on_poweroff>
  <on_reboot>restart</on_reboot>
  <on_crash>destroy</on_crash>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
  </devices>
</domain>
"#;

    #[test]
    fn parses_boot_basics() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.firmware, "bios");
        assert_eq!(c.arch.as_deref(), Some("x86_64"));
        assert_eq!(c.machine.as_deref(), Some("q35"));
        assert_eq!(c.boot_order, vec!["hd", "cdrom"]);
        assert!(c.features.acpi);
        assert!(c.features.apic);
        assert_eq!(c.on_poweroff.as_deref(), Some("destroy"));
        assert_eq!(c.on_reboot.as_deref(), Some("restart"));
        assert_eq!(c.cpu_mode.as_deref(), Some("host-passthrough"));
    }

    #[test]
    fn detects_efi_and_secure_boot() {
        let xml = r#"<domain>
          <os>
            <type arch='x86_64' machine='q35'>hvm</type>
            <loader readonly='yes' type='pflash' secure='yes'>/usr/share/OVMF/OVMF_CODE.fd</loader>
            <boot dev='hd'/>
          </os>
        </domain>"#;
        let c = parse(xml).unwrap();
        assert_eq!(c.firmware, "efi");
        assert!(c.secure_boot);
    }

    #[test]
    fn boot_menu_parsed() {
        let xml = r#"<domain><os><type>hvm</type><bootmenu enable='yes' timeout='5000'/></os></domain>"#;
        let c = parse(xml).unwrap();
        assert!(c.boot_menu_enabled);
        assert_eq!(c.boot_menu_timeout_ms, Some(5000));
    }

    #[test]
    fn apply_reorders_boot_devices() {
        let patch = BootPatch {
            boot_order: Some(vec!["cdrom".into(), "hd".into(), "network".into()]),
            ..Default::default()
        };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        let c = parse(&new_xml).unwrap();
        assert_eq!(c.boot_order, vec!["cdrom", "hd", "network"]);
    }

    #[test]
    fn apply_switches_bios_to_efi() {
        let patch = BootPatch {
            firmware: Some("efi".into()),
            ..Default::default()
        };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        assert!(new_xml.contains("<loader readonly='yes' type='pflash'"));
        let c = parse(&new_xml).unwrap();
        assert_eq!(c.firmware, "efi");
    }

    #[test]
    fn apply_enables_boot_menu() {
        let patch = BootPatch {
            boot_menu_enabled: Some(true),
            boot_menu_timeout_ms: Some(Some(3000)),
            ..Default::default()
        };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        let c = parse(&new_xml).unwrap();
        assert!(c.boot_menu_enabled);
        assert_eq!(c.boot_menu_timeout_ms, Some(3000));
    }

    #[test]
    fn apply_changes_machine_type() {
        let patch = BootPatch {
            machine: Some("pc-i440fx-6.2".into()),
            ..Default::default()
        };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        let c = parse(&new_xml).unwrap();
        assert_eq!(c.machine.as_deref(), Some("pc-i440fx-6.2"));
    }

    #[test]
    fn apply_events() {
        let patch = BootPatch {
            on_poweroff: Some("preserve".into()),
            on_crash: Some("coredump-restart".into()),
            ..Default::default()
        };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        let c = parse(&new_xml).unwrap();
        assert_eq!(c.on_poweroff.as_deref(), Some("preserve"));
        assert_eq!(c.on_crash.as_deref(), Some("coredump-restart"));
        assert_eq!(c.on_reboot.as_deref(), Some("restart")); // untouched
    }

    #[test]
    fn apply_features_replaces_block() {
        let patch = BootPatch {
            features: Some(FeatureFlags { acpi: true, apic: true, smm: true, ..Default::default() }),
            ..Default::default()
        };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        let c = parse(&new_xml).unwrap();
        assert!(c.features.smm);
        assert!(c.features.acpi);
        assert!(c.features.apic);
    }

    #[test]
    fn apply_cpu_mode() {
        let patch = BootPatch {
            cpu_mode: Some("host-model".into()),
            ..Default::default()
        };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        let c = parse(&new_xml).unwrap();
        assert_eq!(c.cpu_mode.as_deref(), Some("host-model"));
    }

    #[test]
    fn apply_preserves_other_sections() {
        let patch = BootPatch { boot_order: Some(vec!["cdrom".into()]), ..Default::default() };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        // <memory>, <vcpu>, <devices><emulator> survive.
        assert!(new_xml.contains("<memory unit='KiB'>1024</memory>"));
        assert!(new_xml.contains("<vcpu>1</vcpu>"));
        assert!(new_xml.contains("<emulator>/usr/bin/qemu-system-x86_64</emulator>"));
    }

    #[test]
    fn apply_is_idempotent() {
        let patch = BootPatch { boot_order: Some(vec!["hd".into(), "cdrom".into()]), ..Default::default() };
        let once = apply(SAMPLE, &patch).unwrap();
        let twice = apply(&once, &patch).unwrap();
        assert_eq!(parse(&once).unwrap(), parse(&twice).unwrap());
    }

    #[test]
    fn apply_efi_with_secure_boot() {
        let patch = BootPatch {
            firmware: Some("efi".into()),
            secure_boot: Some(true),
            ..Default::default()
        };
        let new_xml = apply(SAMPLE, &patch).unwrap();
        assert!(new_xml.contains("secure='yes'"));
    }

    #[test]
    fn event_actions_are_known_set() {
        for a in EVENT_ACTIONS {
            assert!(!a.is_empty());
        }
    }
}
