//! Virtio-adjacent device batch: TPM, RNG, watchdog, panic, memballoon,
//! vsock, and IOMMU.
//!
//! Each device type has:
//! - a Config struct
//! - `parse_<name>` that extracts it from a domain XML (Vec for RNG,
//!   Option for the others — they are one-per-domain)
//! - `build_<name>_xml` that serialises a Config back to a `<...>` element
//! - `apply_set_<name>` that swaps (or removes) the one-per-domain entry
//!   in the `<devices>` block in place.
//!
//! Constraints (validated here, not at the XML layer):
//! - Only one TPM per domain. TPM cannot be hotplugged — persistent only.
//! - Only one watchdog per domain. Persistent only.
//! - vsock CID must be >= 3 (0, 1, 2 are reserved: hypervisor / local / host).
//! - Balloon autodeflate requires a virtio model.
//! - Panic notifier model must match the domain architecture (e.g. `isa`
//!   for x86, `pseries` for ppc64). We do not enforce arch matching here;
//!   the UI restricts choices using DomainCaps.
//! - IOMMU is persistent only; requires specific machine types (q35 for
//!   intel, virt for smmuv3).

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

// ═══════════════════════════════════════════════════════════════════════
//                          Config types
// ═══════════════════════════════════════════════════════════════════════

/// TPM device. One per domain. Persistent only — hotplug unsupported.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TpmConfig {
    /// `tpm-tis`, `tpm-crb`, or `tpm-spapr`.
    pub model: String,
    /// `passthrough`, `emulator`, or `external`.
    pub backend_model: String,
    /// `1.2` or `2.0`. Only meaningful for emulator backend.
    pub backend_version: Option<String>,
    /// For passthrough backend: host char device path (e.g. `/dev/tpm0`).
    pub source_path: Option<String>,
}

/// Random number generator. Multiple allowed per domain. Hotpluggable.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RngConfig {
    /// `virtio`, `virtio-transitional`, or `virtio-non-transitional`.
    pub model: String,
    /// `random`, `builtin`, or `egd`.
    pub backend_model: String,
    /// Path or EGD source. `/dev/urandom` default for `random`.
    pub source_path: Option<String>,
    /// Rate limit period in milliseconds (optional).
    pub rate_period_ms: Option<u32>,
    /// Rate limit bytes per period (optional). Both must be set or neither.
    pub rate_bytes: Option<u32>,
}

/// Watchdog device. One per domain. Persistent only.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WatchdogConfig {
    /// `i6300esb`, `ib700`, `itco`, or `diag288`.
    pub model: String,
    /// `reset`, `shutdown`, `poweroff`, `pause`, `dump`, `inject-nmi`, `none`.
    pub action: String,
}

/// Panic notifier. Persistent only.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PanicConfig {
    /// `isa`, `pseries`, `hyperv`, `s390`, or `pvpanic`.
    pub model: String,
}

/// Memory balloon. Stats period hot-settable via SetMemoryStatsPeriod;
/// model change persistent only.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BalloonConfig {
    /// `virtio`, `virtio-transitional`, `virtio-non-transitional`, or `none`.
    pub model: String,
    /// `autodeflate`. Requires virtio model.
    pub autodeflate: bool,
    /// Free-page reporting (virtio-balloon PAGE_REPORTING).
    pub freepage_reporting: bool,
    /// Period for memory stats collection, seconds. None = unset.
    pub stats_period_secs: Option<u32>,
}

/// Vsock socket. At most one per domain. Hotpluggable.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VsockConfig {
    /// Context ID. Must be >= 3 (0/1/2 are reserved).
    pub cid: u32,
    /// `virtio`, `virtio-transitional`, `virtio-non-transitional`.
    pub model: String,
    /// `auto='yes'` lets libvirt pick a free CID.
    pub auto_cid: bool,
}

/// IOMMU device. Persistent only.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct IommuConfig {
    /// `intel`, `smmuv3`, or `virtio`.
    pub model: String,
    pub driver_intremap: bool,
    pub driver_caching_mode: bool,
    pub driver_eim: bool,
    pub driver_iotlb: bool,
}

/// Aggregate snapshot of all virtio-adjacent devices for a domain.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtioDevicesSnapshot {
    pub tpm: Option<TpmConfig>,
    pub rngs: Vec<RngConfig>,
    pub watchdog: Option<WatchdogConfig>,
    pub panic: Option<PanicConfig>,
    pub balloon: Option<BalloonConfig>,
    pub vsock: Option<VsockConfig>,
    pub iommu: Option<IommuConfig>,
}

// ═══════════════════════════════════════════════════════════════════════
//                          Validation
// ═══════════════════════════════════════════════════════════════════════

pub const VSOCK_MIN_CID: u32 = 3;

pub const TPM_MODELS: &[&str] = &["tpm-tis", "tpm-crb", "tpm-spapr"];
pub const TPM_BACKENDS: &[&str] = &["passthrough", "emulator", "external"];
pub const WATCHDOG_MODELS: &[&str] = &["i6300esb", "ib700", "itco", "diag288"];
pub const WATCHDOG_ACTIONS: &[&str] = &[
    "reset", "shutdown", "poweroff", "pause", "dump", "inject-nmi", "none",
];
pub const PANIC_MODELS: &[&str] = &["isa", "pseries", "hyperv", "s390", "pvpanic"];
pub const RNG_BACKEND_MODELS: &[&str] = &["random", "builtin", "egd"];
pub const IOMMU_MODELS: &[&str] = &["intel", "smmuv3", "virtio"];

impl VsockConfig {
    pub fn validate(&self) -> Result<(), VirtManagerError> {
        if !self.auto_cid && self.cid < VSOCK_MIN_CID {
            return Err(VirtManagerError::OperationFailed {
                operation: "validateVsock".into(),
                reason: format!(
                    "vsock CID {} is reserved — must be >= {}",
                    self.cid, VSOCK_MIN_CID
                ),
            });
        }
        Ok(())
    }
}

impl WatchdogConfig {
    pub fn validate(&self) -> Result<(), VirtManagerError> {
        if !WATCHDOG_ACTIONS.contains(&self.action.as_str()) {
            return Err(VirtManagerError::OperationFailed {
                operation: "validateWatchdog".into(),
                reason: format!("unknown watchdog action: {}", self.action),
            });
        }
        Ok(())
    }
}

impl BalloonConfig {
    pub fn validate(&self) -> Result<(), VirtManagerError> {
        if self.autodeflate && !self.model.starts_with("virtio") {
            return Err(VirtManagerError::OperationFailed {
                operation: "validateBalloon".into(),
                reason: "autodeflate requires a virtio balloon model".into(),
            });
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════
//                          Parsing
// ═══════════════════════════════════════════════════════════════════════

/// Walk the domain XML and yield (name, attrs, inner_text) per top-level
/// device under `<devices>`. Avoids pulling in a full DOM.
fn each_device(xml: &str) -> Result<Vec<(String, Vec<(String, String)>, String)>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut in_devices = false;
    let mut depth_in_devices: i32 = 0;
    let mut current_name: Option<String> = None;
    let mut current_attrs: Vec<(String, String)> = Vec::new();
    let mut current_start: Option<usize> = None;
    let mut out: Vec<(String, Vec<(String, String)>, String)> = Vec::new();

    loop {
        let pos_before = r.buffer_position() as usize;
        let ev = r
            .read_event_into(&mut buf)
            .map_err(|e| VirtManagerError::XmlParsingFailed { reason: e.to_string() })?;
        let pos_after = r.buffer_position() as usize;

        match ev {
            Event::Eof => break,
            Event::Start(e) => {
                let n = utf8_name(&e);
                if n == "devices" && !in_devices {
                    in_devices = true;
                } else if in_devices {
                    if depth_in_devices == 0 {
                        current_name = Some(n);
                        current_attrs = attrs(&e);
                        current_start = Some(pos_after);
                    }
                    depth_in_devices += 1;
                }
            }
            Event::Empty(e) if in_devices && depth_in_devices == 0 => {
                let n = utf8_name(&e);
                out.push((n, attrs(&e), String::new()));
            }
            Event::End(e) => {
                let n = utf8_name_end(&e);
                if in_devices && depth_in_devices > 0 {
                    depth_in_devices -= 1;
                    if depth_in_devices == 0 {
                        if let (Some(nm), Some(s)) = (current_name.take(), current_start.take()) {
                            let inner = xml.get(s..pos_before).unwrap_or("").to_string();
                            out.push((nm, std::mem::take(&mut current_attrs), inner));
                        }
                    }
                } else if in_devices && depth_in_devices == 0 && n == "devices" {
                    // outer devices closed; done
                    let _ = in_devices;
                    break;
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn utf8_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn utf8_name_end(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .filter_map(|a| a.ok())
        .map(|a| {
            (
                String::from_utf8_lossy(a.key.as_ref()).to_string(),
                a.unescape_value().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn attr<'a>(list: &'a [(String, String)], key: &str) -> Option<&'a str> {
    list.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

// ── TPM ─────────────────────────────────────────────────────────────────

pub fn parse_tpm(xml: &str) -> Result<Option<TpmConfig>, VirtManagerError> {
    for (name, a, inner) in each_device(xml)? {
        if name != "tpm" { continue; }
        let model = attr(&a, "model").unwrap_or("tpm-tis").to_string();
        let mut backend_model = String::new();
        let mut backend_version = None;
        let mut source_path: Option<String> = None;
        let mut r = Reader::from_str(&inner);
        r.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut in_backend = false;
        let mut backend_text = String::new();
        loop {
            let ev = r.read_event_into(&mut buf);
            match ev {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Start(e)) => {
                    let nm = utf8_name(&e);
                    if nm == "backend" {
                        in_backend = true;
                        let ba = attrs(&e);
                        backend_model = attr(&ba, "type").or_else(|| attr(&ba, "model"))
                            .unwrap_or("passthrough").to_string();
                        backend_version = attr(&ba, "version").map(String::from);
                    } else if in_backend && nm == "device" {
                        let da = attrs(&e);
                        source_path = attr(&da, "path").map(String::from);
                    }
                }
                Ok(Event::Empty(e)) => {
                    let nm = utf8_name(&e);
                    if nm == "backend" {
                        let ba = attrs(&e);
                        backend_model = attr(&ba, "type").or_else(|| attr(&ba, "model"))
                            .unwrap_or("passthrough").to_string();
                        backend_version = attr(&ba, "version").map(String::from);
                    } else if in_backend && nm == "device" {
                        let da = attrs(&e);
                        source_path = attr(&da, "path").map(String::from);
                    }
                }
                Ok(Event::Text(t)) if in_backend => {
                    let s = t.unescape().unwrap_or_default().to_string();
                    if !s.trim().is_empty() {
                        backend_text.push_str(s.trim());
                    }
                }
                Ok(Event::End(e)) if utf8_name_end(&e) == "backend" => {
                    in_backend = false;
                }
                _ => {}
            }
            buf.clear();
        }
        if source_path.is_none() && !backend_text.is_empty() && backend_model == "passthrough" {
            source_path = Some(backend_text);
        }
        return Ok(Some(TpmConfig { model, backend_model, backend_version, source_path }));
    }
    Ok(None)
}

pub fn build_tpm_xml(cfg: &TpmConfig) -> String {
    let mut s = format!("<tpm model='{}'>\n", escape_xml(&cfg.model));
    match cfg.backend_model.as_str() {
        "emulator" => {
            if let Some(v) = &cfg.backend_version {
                s.push_str(&format!(
                    "      <backend type='emulator' version='{}'/>\n",
                    escape_xml(v)
                ));
            } else {
                s.push_str("      <backend type='emulator'/>\n");
            }
        }
        "external" => {
            s.push_str("      <backend type='external'>\n");
            if let Some(p) = &cfg.source_path {
                s.push_str(&format!(
                    "        <source type='unix' mode='connect' path='{}'/>\n",
                    escape_xml(p)
                ));
            }
            s.push_str("      </backend>\n");
        }
        _ => {
            s.push_str("      <backend type='passthrough'>\n");
            let path = cfg.source_path.as_deref().unwrap_or("/dev/tpm0");
            s.push_str(&format!("        <device path='{}'/>\n", escape_xml(path)));
            s.push_str("      </backend>\n");
        }
    }
    s.push_str("    </tpm>");
    s
}

// ── RNG ─────────────────────────────────────────────────────────────────

pub fn parse_rngs(xml: &str) -> Result<Vec<RngConfig>, VirtManagerError> {
    let mut out = Vec::new();
    for (name, a, inner) in each_device(xml)? {
        if name != "rng" { continue; }
        let model = attr(&a, "model").unwrap_or("virtio").to_string();
        let mut backend_model = "random".to_string();
        let mut source_path: Option<String> = None;
        let mut rate_period_ms = None;
        let mut rate_bytes = None;

        let mut r = Reader::from_str(&inner);
        r.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut in_backend = false;
        let mut backend_text = String::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Start(e)) => {
                    let nm = utf8_name(&e);
                    if nm == "backend" {
                        in_backend = true;
                        let ba = attrs(&e);
                        backend_model = attr(&ba, "model").unwrap_or("random").to_string();
                    }
                }
                Ok(Event::Empty(e)) => {
                    let nm = utf8_name(&e);
                    if nm == "backend" {
                        let ba = attrs(&e);
                        backend_model = attr(&ba, "model").unwrap_or("random").to_string();
                    } else if nm == "rate" {
                        let ra = attrs(&e);
                        rate_period_ms = attr(&ra, "period").and_then(|s| s.parse().ok());
                        rate_bytes = attr(&ra, "bytes").and_then(|s| s.parse().ok());
                    }
                }
                Ok(Event::Text(t)) if in_backend => {
                    let s = t.unescape().unwrap_or_default().to_string();
                    if !s.trim().is_empty() {
                        backend_text.push_str(s.trim());
                    }
                }
                Ok(Event::End(e)) if utf8_name_end(&e) == "backend" => {
                    in_backend = false;
                }
                _ => {}
            }
            buf.clear();
        }
        if !backend_text.is_empty() {
            source_path = Some(backend_text);
        }
        out.push(RngConfig {
            model,
            backend_model,
            source_path,
            rate_period_ms,
            rate_bytes,
        });
    }
    Ok(out)
}

pub fn build_rng_xml(cfg: &RngConfig) -> String {
    let mut s = format!("<rng model='{}'>\n", escape_xml(&cfg.model));
    match cfg.backend_model.as_str() {
        "builtin" => {
            s.push_str("      <backend model='builtin'/>\n");
        }
        "egd" => {
            s.push_str("      <backend model='egd' type='unix'>\n");
            if let Some(p) = &cfg.source_path {
                s.push_str(&format!(
                    "        <source mode='connect' path='{}'/>\n",
                    escape_xml(p)
                ));
            }
            s.push_str("      </backend>\n");
        }
        _ => {
            let path = cfg.source_path.as_deref().unwrap_or("/dev/urandom");
            s.push_str(&format!(
                "      <backend model='random'>{}</backend>\n",
                escape_xml(path)
            ));
        }
    }
    if let (Some(period), Some(bytes)) = (cfg.rate_period_ms, cfg.rate_bytes) {
        s.push_str(&format!(
            "      <rate period='{}' bytes='{}'/>\n",
            period, bytes
        ));
    }
    s.push_str("    </rng>");
    s
}

// ── Watchdog ────────────────────────────────────────────────────────────

pub fn parse_watchdog(xml: &str) -> Result<Option<WatchdogConfig>, VirtManagerError> {
    for (name, a, _inner) in each_device(xml)? {
        if name != "watchdog" { continue; }
        return Ok(Some(WatchdogConfig {
            model: attr(&a, "model").unwrap_or("i6300esb").to_string(),
            action: attr(&a, "action").unwrap_or("reset").to_string(),
        }));
    }
    Ok(None)
}

pub fn build_watchdog_xml(cfg: &WatchdogConfig) -> String {
    format!(
        "<watchdog model='{}' action='{}'/>",
        escape_xml(&cfg.model),
        escape_xml(&cfg.action)
    )
}

// ── Panic ───────────────────────────────────────────────────────────────

pub fn parse_panic(xml: &str) -> Result<Option<PanicConfig>, VirtManagerError> {
    for (name, a, _inner) in each_device(xml)? {
        if name != "panic" { continue; }
        return Ok(Some(PanicConfig {
            model: attr(&a, "model").unwrap_or("isa").to_string(),
        }));
    }
    Ok(None)
}

pub fn build_panic_xml(cfg: &PanicConfig) -> String {
    format!("<panic model='{}'/>", escape_xml(&cfg.model))
}

// ── Balloon ─────────────────────────────────────────────────────────────

pub fn parse_balloon(xml: &str) -> Result<Option<BalloonConfig>, VirtManagerError> {
    for (name, a, inner) in each_device(xml)? {
        if name != "memballoon" { continue; }
        let model = attr(&a, "model").unwrap_or("virtio").to_string();
        let autodeflate = attr(&a, "autodeflate").map(|s| s == "on").unwrap_or(false);
        let freepage_reporting = attr(&a, "freePageReporting")
            .map(|s| s == "on")
            .unwrap_or(false);
        let mut stats_period_secs = None;
        let mut r = Reader::from_str(&inner);
        r.config_mut().trim_text(false);
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                    if utf8_name(&e) == "stats" {
                        let sa = attrs(&e);
                        stats_period_secs = attr(&sa, "period").and_then(|s| s.parse().ok());
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        return Ok(Some(BalloonConfig {
            model,
            autodeflate,
            freepage_reporting,
            stats_period_secs,
        }));
    }
    Ok(None)
}

pub fn build_balloon_xml(cfg: &BalloonConfig) -> String {
    let mut attrs = format!("model='{}'", escape_xml(&cfg.model));
    if cfg.autodeflate {
        attrs.push_str(" autodeflate='on'");
    }
    if cfg.freepage_reporting {
        attrs.push_str(" freePageReporting='on'");
    }
    match cfg.stats_period_secs {
        Some(p) => format!(
            "<memballoon {}>\n      <stats period='{}'/>\n    </memballoon>",
            attrs, p
        ),
        None => format!("<memballoon {}/>", attrs),
    }
}

// ── Vsock ───────────────────────────────────────────────────────────────

pub fn parse_vsock(xml: &str) -> Result<Option<VsockConfig>, VirtManagerError> {
    for (name, a, inner) in each_device(xml)? {
        if name != "vsock" { continue; }
        let model = attr(&a, "model").unwrap_or("virtio").to_string();
        let mut cid = 0u32;
        let mut auto_cid = false;
        let mut r = Reader::from_str(&inner);
        r.config_mut().trim_text(false);
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                    if utf8_name(&e) == "cid" {
                        let ca = attrs(&e);
                        auto_cid = attr(&ca, "auto").map(|s| s == "yes").unwrap_or(false);
                        cid = attr(&ca, "address").and_then(|s| s.parse().ok()).unwrap_or(0);
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        return Ok(Some(VsockConfig { cid, model, auto_cid }));
    }
    Ok(None)
}

pub fn build_vsock_xml(cfg: &VsockConfig) -> String {
    let model = if cfg.model.is_empty() { "virtio" } else { &cfg.model };
    let cid_line = if cfg.auto_cid {
        "      <cid auto='yes'/>\n".to_string()
    } else {
        format!("      <cid auto='no' address='{}'/>\n", cfg.cid)
    };
    format!("<vsock model='{}'>\n{}    </vsock>", escape_xml(model), cid_line)
}

// ── IOMMU ───────────────────────────────────────────────────────────────

pub fn parse_iommu(xml: &str) -> Result<Option<IommuConfig>, VirtManagerError> {
    for (name, a, inner) in each_device(xml)? {
        if name != "iommu" { continue; }
        let model = attr(&a, "model").unwrap_or("intel").to_string();
        let mut cfg = IommuConfig { model, ..Default::default() };
        let mut r = Reader::from_str(&inner);
        r.config_mut().trim_text(false);
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                    if utf8_name(&e) == "driver" {
                        let da = attrs(&e);
                        let yes = |k: &str| attr(&da, k).map(|s| s == "on").unwrap_or(false);
                        cfg.driver_intremap = yes("intremap");
                        cfg.driver_caching_mode = yes("caching_mode");
                        cfg.driver_eim = yes("eim");
                        cfg.driver_iotlb = yes("iotlb");
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        return Ok(Some(cfg));
    }
    Ok(None)
}

pub fn build_iommu_xml(cfg: &IommuConfig) -> String {
    let mut s = format!("<iommu model='{}'>\n", escape_xml(&cfg.model));
    let mut parts: Vec<String> = Vec::new();
    if cfg.driver_intremap { parts.push("intremap='on'".into()); }
    if cfg.driver_caching_mode { parts.push("caching_mode='on'".into()); }
    if cfg.driver_eim { parts.push("eim='on'".into()); }
    if cfg.driver_iotlb { parts.push("iotlb='on'".into()); }
    if !parts.is_empty() {
        s.push_str(&format!("      <driver {}/>\n", parts.join(" ")));
    }
    s.push_str("    </iommu>");
    s
}

// ═══════════════════════════════════════════════════════════════════════
//                 apply_set_* — in-place swap for singletons
// ═══════════════════════════════════════════════════════════════════════

fn devices_region(xml: &str) -> Option<(usize, usize)> {
    let open = xml.find("<devices>")?;
    let open_end = open + "<devices>".len();
    let close = xml[open_end..].find("</devices>")? + open_end;
    Some((open_end, close))
}

fn strip_top_level_element(xml: &str, tag: &str) -> Result<String, VirtManagerError> {
    let (start, end) = match devices_region(xml) {
        Some(r) => r,
        None => return Ok(xml.to_string()),
    };
    let body = &xml[start..end];

    let mut r = Reader::from_str(body);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth: i32 = 0;
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    let mut cur_start: Option<usize> = None;

    loop {
        let pos_before = r.buffer_position() as usize;
        let ev = r
            .read_event_into(&mut buf)
            .map_err(|e| VirtManagerError::XmlParsingFailed { reason: e.to_string() })?;
        let pos_after = r.buffer_position() as usize;
        match ev {
            Event::Eof => break,
            Event::Start(e) => {
                if depth == 0 && utf8_name(&e) == tag {
                    cur_start = Some(pos_before);
                }
                depth += 1;
            }
            Event::Empty(e) => {
                if depth == 0 && utf8_name(&e) == tag {
                    cuts.push((pos_before, pos_after));
                }
            }
            Event::End(_e) => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = cur_start.take() {
                        cuts.push((s, pos_after));
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    if cuts.is_empty() {
        return Ok(xml.to_string());
    }

    let mut out = xml.to_string();
    let mut cuts_abs: Vec<(usize, usize)> = cuts
        .into_iter()
        .map(|(s, e)| (s + start, e + start))
        .collect();
    cuts_abs.sort_by(|a, b| b.0.cmp(&a.0));
    for (s, e) in cuts_abs {
        let mut s2 = s;
        while s2 > 0 {
            let c = out.as_bytes()[s2 - 1];
            if c == b' ' || c == b'\t' { s2 -= 1; continue; }
            break;
        }
        let mut e2 = e;
        if e2 < out.len() && out.as_bytes()[e2] == b'\n' {
            e2 += 1;
        }
        out.replace_range(s2..e2, "");
    }
    Ok(out)
}

fn inject_before_devices_close(xml: &str, fragment: &str) -> Result<String, VirtManagerError> {
    let (_start, end) = devices_region(xml).ok_or_else(|| VirtManagerError::XmlParsingFailed {
        reason: "missing <devices> section".into(),
    })?;
    let mut out = String::with_capacity(xml.len() + fragment.len() + 8);
    out.push_str(&xml[..end]);
    out.push_str("    ");
    out.push_str(fragment);
    out.push('\n');
    out.push_str("  ");
    out.push_str(&xml[end..]);
    Ok(out)
}

fn replace_singleton(
    xml: &str,
    tag: &str,
    new_fragment: Option<&str>,
) -> Result<String, VirtManagerError> {
    let stripped = strip_top_level_element(xml, tag)?;
    match new_fragment {
        Some(frag) => inject_before_devices_close(&stripped, frag),
        None => Ok(stripped),
    }
}

pub fn apply_set_tpm(xml: &str, cfg: Option<&TpmConfig>) -> Result<String, VirtManagerError> {
    let frag = cfg.map(build_tpm_xml);
    replace_singleton(xml, "tpm", frag.as_deref())
}

pub fn apply_set_watchdog(
    xml: &str,
    cfg: Option<&WatchdogConfig>,
) -> Result<String, VirtManagerError> {
    if let Some(c) = cfg { c.validate()?; }
    let frag = cfg.map(build_watchdog_xml);
    replace_singleton(xml, "watchdog", frag.as_deref())
}

pub fn apply_set_panic(xml: &str, cfg: Option<&PanicConfig>) -> Result<String, VirtManagerError> {
    let frag = cfg.map(build_panic_xml);
    replace_singleton(xml, "panic", frag.as_deref())
}

pub fn apply_set_balloon(
    xml: &str,
    cfg: Option<&BalloonConfig>,
) -> Result<String, VirtManagerError> {
    if let Some(c) = cfg { c.validate()?; }
    let frag = cfg.map(build_balloon_xml);
    replace_singleton(xml, "memballoon", frag.as_deref())
}

pub fn apply_set_vsock(xml: &str, cfg: Option<&VsockConfig>) -> Result<String, VirtManagerError> {
    if let Some(c) = cfg { c.validate()?; }
    let frag = cfg.map(build_vsock_xml);
    replace_singleton(xml, "vsock", frag.as_deref())
}

pub fn apply_set_iommu(xml: &str, cfg: Option<&IommuConfig>) -> Result<String, VirtManagerError> {
    let frag = cfg.map(build_iommu_xml);
    replace_singleton(xml, "iommu", frag.as_deref())
}

// ═══════════════════════════════════════════════════════════════════════
//                                Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<domain type='kvm'>
  <name>t</name>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
    <watchdog model='itco' action='reset'>
      <alias name='watchdog0'/>
    </watchdog>
    <memballoon model='virtio'>
      <alias name='balloon0'/>
    </memballoon>
    <rng model='virtio'>
      <backend model='random'>/dev/urandom</backend>
      <alias name='rng0'/>
    </rng>
  </devices>
</domain>
"#;

    #[test]
    fn parses_watchdog() {
        let w = parse_watchdog(SAMPLE).unwrap().unwrap();
        assert_eq!(w.model, "itco");
        assert_eq!(w.action, "reset");
    }

    #[test]
    fn parses_rng_with_source() {
        let rngs = parse_rngs(SAMPLE).unwrap();
        assert_eq!(rngs.len(), 1);
        assert_eq!(rngs[0].model, "virtio");
        assert_eq!(rngs[0].backend_model, "random");
        assert_eq!(rngs[0].source_path.as_deref(), Some("/dev/urandom"));
    }

    #[test]
    fn parses_balloon_virtio_default() {
        let b = parse_balloon(SAMPLE).unwrap().unwrap();
        assert_eq!(b.model, "virtio");
        assert!(!b.autodeflate);
        assert_eq!(b.stats_period_secs, None);
    }

    #[test]
    fn round_trip_watchdog() {
        let before = WatchdogConfig { model: "i6300esb".into(), action: "pause".into() };
        let xml = apply_set_watchdog(SAMPLE, Some(&before)).unwrap();
        let after = parse_watchdog(&xml).unwrap().unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn round_trip_panic() {
        let p = PanicConfig { model: "pvpanic".into() };
        let xml = apply_set_panic(SAMPLE, Some(&p)).unwrap();
        let back = parse_panic(&xml).unwrap().unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn round_trip_tpm_emulator() {
        let t = TpmConfig {
            model: "tpm-crb".into(),
            backend_model: "emulator".into(),
            backend_version: Some("2.0".into()),
            source_path: None,
        };
        let xml = apply_set_tpm(SAMPLE, Some(&t)).unwrap();
        assert!(xml.contains("<tpm model='tpm-crb'>"));
        assert!(xml.contains("version='2.0'"));
        let back = parse_tpm(&xml).unwrap().unwrap();
        assert_eq!(back.model, "tpm-crb");
        assert_eq!(back.backend_model, "emulator");
        assert_eq!(back.backend_version.as_deref(), Some("2.0"));
    }

    #[test]
    fn round_trip_tpm_passthrough() {
        let t = TpmConfig {
            model: "tpm-tis".into(),
            backend_model: "passthrough".into(),
            backend_version: None,
            source_path: Some("/dev/tpm0".into()),
        };
        let xml = apply_set_tpm(SAMPLE, Some(&t)).unwrap();
        let back = parse_tpm(&xml).unwrap().unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn rng_rate_limit_round_trip() {
        let r = RngConfig {
            model: "virtio".into(),
            backend_model: "random".into(),
            source_path: Some("/dev/urandom".into()),
            rate_period_ms: Some(2000),
            rate_bytes: Some(1024),
        };
        let xml = build_rng_xml(&r);
        assert!(xml.contains("<rate period='2000' bytes='1024'/>"));
        let domain = format!("<domain><devices>{}</devices></domain>", xml);
        let back = parse_rngs(&domain).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].rate_period_ms, Some(2000));
        assert_eq!(back[0].rate_bytes, Some(1024));
    }

    #[test]
    fn multiple_rngs_preserved_on_parse() {
        let xml = r#"<domain><devices>
            <rng model='virtio'><backend model='random'>/dev/urandom</backend></rng>
            <rng model='virtio'><backend model='builtin'/></rng>
        </devices></domain>"#;
        let rngs = parse_rngs(xml).unwrap();
        assert_eq!(rngs.len(), 2);
        assert_eq!(rngs[0].backend_model, "random");
        assert_eq!(rngs[1].backend_model, "builtin");
    }

    #[test]
    fn vsock_cid_validation_rejects_reserved() {
        for bad in [0u32, 1, 2] {
            let v = VsockConfig { cid: bad, model: "virtio".into(), auto_cid: false };
            assert!(v.validate().is_err(), "cid={} should be rejected", bad);
        }
        let ok = VsockConfig { cid: 3, model: "virtio".into(), auto_cid: false };
        assert!(ok.validate().is_ok());
        let auto = VsockConfig { cid: 0, model: "virtio".into(), auto_cid: true };
        assert!(auto.validate().is_ok());
    }

    #[test]
    fn vsock_round_trip() {
        let v = VsockConfig { cid: 42, model: "virtio".into(), auto_cid: false };
        let xml = apply_set_vsock(SAMPLE, Some(&v)).unwrap();
        assert!(xml.contains("address='42'"));
        let back = parse_vsock(&xml).unwrap().unwrap();
        assert_eq!(back.cid, 42);
        assert_eq!(back.model, "virtio");
    }

    #[test]
    fn watchdog_action_enum_rejects_unknown() {
        let bad = WatchdogConfig { model: "i6300esb".into(), action: "explode".into() };
        assert!(bad.validate().is_err());
        for a in WATCHDOG_ACTIONS {
            let ok = WatchdogConfig { model: "i6300esb".into(), action: (*a).into() };
            assert!(ok.validate().is_ok());
        }
    }

    #[test]
    fn balloon_autodeflate_requires_virtio() {
        let bad = BalloonConfig {
            model: "none".into(),
            autodeflate: true,
            freepage_reporting: false,
            stats_period_secs: None,
        };
        assert!(bad.validate().is_err());
        let ok = BalloonConfig {
            model: "virtio".into(),
            autodeflate: true,
            freepage_reporting: false,
            stats_period_secs: None,
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn remove_watchdog() {
        let xml = apply_set_watchdog(SAMPLE, None).unwrap();
        assert!(parse_watchdog(&xml).unwrap().is_none());
        assert!(xml.contains("<memballoon"));
        assert!(xml.contains("<rng"));
        assert!(xml.contains("<emulator>"));
    }

    #[test]
    fn xml_escape_injection_safe() {
        let t = TpmConfig {
            model: "tpm-tis'><evil/>".into(),
            backend_model: "passthrough".into(),
            backend_version: None,
            source_path: Some("/dev/'><pwn/>".into()),
        };
        let frag = build_tpm_xml(&t);
        assert!(!frag.contains("<evil"));
        assert!(!frag.contains("<pwn"));
        assert!(frag.contains("&apos;"));
    }

    #[test]
    fn iommu_round_trip() {
        let i = IommuConfig {
            model: "intel".into(),
            driver_intremap: true,
            driver_caching_mode: true,
            driver_eim: true,
            driver_iotlb: false,
        };
        let xml = apply_set_iommu(SAMPLE, Some(&i)).unwrap();
        assert!(xml.contains("<iommu model='intel'>"));
        assert!(xml.contains("intremap='on'"));
        let back = parse_iommu(&xml).unwrap().unwrap();
        assert_eq!(back, i);
    }

    #[test]
    fn balloon_with_stats_period() {
        let b = BalloonConfig {
            model: "virtio".into(),
            autodeflate: false,
            freepage_reporting: true,
            stats_period_secs: Some(10),
        };
        let xml = apply_set_balloon(SAMPLE, Some(&b)).unwrap();
        assert!(xml.contains("freePageReporting='on'"));
        assert!(xml.contains("<stats period='10'/>"));
        let back = parse_balloon(&xml).unwrap().unwrap();
        assert_eq!(back.stats_period_secs, Some(10));
        assert!(back.freepage_reporting);
    }

    #[test]
    fn replace_preserves_other_devices() {
        let w = WatchdogConfig { model: "i6300esb".into(), action: "shutdown".into() };
        let xml = apply_set_watchdog(SAMPLE, Some(&w)).unwrap();
        assert!(xml.contains("<memballoon model='virtio'>"));
        assert!(xml.contains("<rng model='virtio'>"));
        assert!(xml.contains("<backend model='random'>/dev/urandom</backend>"));
        let count = xml.matches("<watchdog").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn apply_idempotent() {
        let w = WatchdogConfig { model: "i6300esb".into(), action: "reset".into() };
        let a = apply_set_watchdog(SAMPLE, Some(&w)).unwrap();
        let b = apply_set_watchdog(&a, Some(&w)).unwrap();
        assert_eq!(parse_watchdog(&a).unwrap(), parse_watchdog(&b).unwrap());
        assert_eq!(a.matches("<watchdog").count(), 1);
        assert_eq!(b.matches("<watchdog").count(), 1);
    }

    #[test]
    fn inject_into_empty_devices() {
        let xml = "<domain><devices></devices></domain>";
        let p = PanicConfig { model: "pvpanic".into() };
        let out = apply_set_panic(xml, Some(&p)).unwrap();
        assert!(out.contains("<panic model='pvpanic'/>"));
    }

    #[test]
    fn snapshot_shape() {
        let snap = VirtioDevicesSnapshot {
            tpm: None,
            rngs: parse_rngs(SAMPLE).unwrap(),
            watchdog: parse_watchdog(SAMPLE).unwrap(),
            panic: parse_panic(SAMPLE).unwrap(),
            balloon: parse_balloon(SAMPLE).unwrap(),
            vsock: parse_vsock(SAMPLE).unwrap(),
            iommu: parse_iommu(SAMPLE).unwrap(),
        };
        assert_eq!(snap.rngs.len(), 1);
        assert!(snap.watchdog.is_some());
        assert!(snap.panic.is_none());
        assert!(snap.balloon.is_some());
        assert!(snap.vsock.is_none());
    }
}
