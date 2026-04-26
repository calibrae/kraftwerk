//! Memory hotplug: `<maxMemory slots="N">` boot-time configuration plus
//! per-DIMM `<memory model="dimm">` device attach for live grow.
//!
//! Splitting this out of `domain_config` because:
//! - the slots/max bookkeeping is its own narrow concern
//! - dimm device XML is built independently of the rest of the domain
//! - keeps `connection.rs` focused on libvirt API wrappers, not XML
//!
//! Constraints (libvirt):
//! - `<maxMemory slots>` can only change when the VM is shut off
//! - The total of base `<memory>` + attached DIMMs must not exceed
//!   `<maxMemory>`
//! - DIMM size should be a multiple of the memory block size for the
//!   guest arch (typically 2 MiB for x86_64). We don't enforce — libvirt
//!   does and surfaces a clear error.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaxMemoryConfig {
    /// Total maximum memory (boot + hotplug headroom) in KiB.
    pub max_kib: u64,
    /// Number of memory slots the guest is told to expose.
    pub slots: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimmInfo {
    pub size_kib: u64,
    /// NUMA target node for this DIMM. None on UMA guests.
    pub node: Option<u32>,
}

/// Parse `<maxMemory slots='N' unit='KiB'>VAL</maxMemory>` from a domain
/// XML. Returns None if the element is absent (no hotplug configured).
pub fn parse_max_memory(xml: &str) -> Option<MaxMemoryConfig> {
    let i = xml.find("<maxMemory")?;
    let rest = &xml[i..];
    let close = rest.find(">")? + 1;
    let header = &rest[..close];
    // Slots attribute
    let slots = extract_attr(header, "slots").and_then(|s| s.parse().ok()).unwrap_or(1);
    // Unit (default KiB per libvirt schema)
    let unit = extract_attr(header, "unit").unwrap_or_else(|| "KiB".into());
    // Body until </maxMemory>
    let body_start = i + close;
    let body_end = xml[body_start..].find("</maxMemory>")? + body_start;
    let raw = xml[body_start..body_end].trim();
    let value: u64 = raw.parse().ok()?;
    let max_kib = match unit.as_str() {
        "B" | "bytes" => value / 1024,
        "KiB" | "K" | "k" => value,
        "MiB" | "M" | "m" => value * 1024,
        "GiB" | "G" | "g" => value * 1024 * 1024,
        _ => value,
    };
    Some(MaxMemoryConfig { max_kib, slots })
}

/// Count `<memory model="dimm">` device entries in a domain XML.
/// Used for the UI to show "X / N slots used" without parsing each DIMM.
pub fn count_dimms(xml: &str) -> u32 {
    // Exact-match on the common opening sequence; the dimm model tag
    // can also include `access`, so match the prefix.
    xml.matches("<memory model='dimm'")
        .count()
        .saturating_add(xml.matches("<memory model=\"dimm\"").count()) as u32
}

/// Build a `<memory model='dimm'>` XML fragment to attach.
/// `node` is the target NUMA node; pass None for UMA guests.
pub fn build_dimm_xml(size_kib: u64, node: Option<u32>) -> String {
    match node {
        Some(n) => format!(
            "<memory model='dimm'>\n  <target>\n    <size unit='KiB'>{size_kib}</size>\n    <node>{n}</node>\n  </target>\n</memory>",
        ),
        None => format!(
            "<memory model='dimm'>\n  <target>\n    <size unit='KiB'>{size_kib}</size>\n  </target>\n</memory>",
        ),
    }
}

/// Replace (or insert) the `<maxMemory>` element in a domain XML.
/// Used by `set_max_memory_slots` to update the persistent config.
pub fn apply_max_memory(xml: &str, cfg: &MaxMemoryConfig) -> String {
    let new = format!(
        "<maxMemory slots='{}' unit='KiB'>{}</maxMemory>",
        cfg.slots, cfg.max_kib
    );
    // If an existing element is present, replace it.
    if let (Some(s), Some(e)) = (xml.find("<maxMemory"), xml.find("</maxMemory>")) {
        let end = e + "</maxMemory>".len();
        return format!("{}{}{}", &xml[..s], new, &xml[end..]);
    }
    // Otherwise insert immediately before <memory>.
    if let Some(i) = xml.find("<memory") {
        // libvirt expects maxMemory before memory; preserve indentation
        // by inserting on a new line with the same leading whitespace
        // as the <memory> element.
        let line_start = xml[..i].rfind('\n').map(|n| n + 1).unwrap_or(0);
        let indent = &xml[line_start..i];
        return format!("{}{}{}\n{}{}", &xml[..line_start], indent, new, indent, &xml[i..]);
    }
    // Fallback: append before </domain>.
    if let Some(i) = xml.find("</domain>") {
        return format!("{}  {}\n{}", &xml[..i], new, &xml[i..]);
    }
    xml.to_string()
}

fn extract_attr(s: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let i = s.find(&needle)? + needle.len();
    let bytes = s.as_bytes();
    let q = *bytes.get(i)? as char;
    if q != '"' && q != '\'' {
        return None;
    }
    let after = &s[i + 1..];
    let end = after.find(q)?;
    Some(after[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_max_memory_with_slots_kib() {
        let xml = "<domain><maxMemory slots='16' unit='KiB'>33554432</maxMemory></domain>";
        let m = parse_max_memory(xml).unwrap();
        assert_eq!(m.slots, 16);
        assert_eq!(m.max_kib, 33_554_432);
    }

    #[test]
    fn parses_max_memory_in_gib() {
        let xml = "<domain><maxMemory slots='4' unit='GiB'>32</maxMemory></domain>";
        let m = parse_max_memory(xml).unwrap();
        assert_eq!(m.max_kib, 32 * 1024 * 1024);
    }

    #[test]
    fn returns_none_when_not_present() {
        let xml = "<domain><memory unit='KiB'>2097152</memory></domain>";
        assert!(parse_max_memory(xml).is_none());
    }

    #[test]
    fn counts_dimm_devices() {
        let xml = r#"<devices>
            <memory model='dimm'><target><size unit='KiB'>524288</size></target></memory>
            <memory model="dimm"><target><size unit='KiB'>524288</size></target></memory>
            <memory model='nvdimm'><target/></memory>
        </devices>"#;
        assert_eq!(count_dimms(xml), 2);
    }

    #[test]
    fn build_dimm_xml_with_and_without_node() {
        let with = build_dimm_xml(524_288, Some(0));
        assert!(with.contains("<size unit='KiB'>524288</size>"));
        assert!(with.contains("<node>0</node>"));
        let without = build_dimm_xml(524_288, None);
        assert!(without.contains("<size unit='KiB'>524288</size>"));
        assert!(!without.contains("<node>"));
    }

    #[test]
    fn apply_max_memory_replaces_existing() {
        let xml = "<domain>\n  <maxMemory slots='2' unit='KiB'>4194304</maxMemory>\n  <memory unit='KiB'>2097152</memory>\n</domain>";
        let cfg = MaxMemoryConfig { max_kib: 16_777_216, slots: 8 };
        let out = apply_max_memory(xml, &cfg);
        assert!(out.contains("slots='8'"));
        assert!(out.contains("16777216"));
        assert!(!out.contains("4194304"));
        // <memory> still present
        assert!(out.contains("<memory unit='KiB'>2097152</memory>"));
    }

    #[test]
    fn apply_max_memory_inserts_when_absent() {
        let xml = "<domain>\n  <memory unit='KiB'>2097152</memory>\n</domain>";
        let cfg = MaxMemoryConfig { max_kib: 8_388_608, slots: 4 };
        let out = apply_max_memory(xml, &cfg);
        assert!(out.contains("<maxMemory slots='4' unit='KiB'>8388608</maxMemory>"));
        // maxMemory must come before memory
        let mi = out.find("<maxMemory").unwrap();
        let me = out.find("<memory ").unwrap();
        assert!(mi < me);
    }
}
