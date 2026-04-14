//! Advanced CPU / memory / tuning editor (Round I).
//!
//! Parses and patches the `<cpu>`, `<vcpu>`, `<cputune>`, `<memtune>`,
//! `<cpu><numa>`, `<memoryBacking><hugepages>` and `<iothreads>` sections
//! of a domain XML. Follows the same "mutate in place" streaming strategy
//! used by `boot_config.rs::replace_element_block` so untouched sections
//! round-trip exactly.
//!
//! **Scope:** advanced tuning rarely needed but expected for parity with
//! virt-manager. Everything here applies to the persistent definition
//! only — most of these settings require a restart, and the few that
//! support live-hotplug (vCPU count, iothread count) have dedicated
//! connection methods that call the corresponding libvirt C API.

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

// ═════════════════════════════════════════════════════════════════════
// Types
// ═════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CpuConfig {
    pub mode: String,
    pub model: Option<String>,
    pub check: Option<String>,
    pub migratable: Option<bool>,
    pub cache: Option<CpuCache>,
    pub topology: Option<CpuTopology>,
    pub features: Vec<CpuFeature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CpuCache {
    pub mode: String,
    pub level: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CpuTopology {
    pub sockets: u32,
    pub dies: u32,
    pub cores: u32,
    pub threads: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CpuFeature {
    pub name: String,
    pub policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VcpuConfig {
    pub max: u32,
    pub current: u32,
    pub placement: Option<String>,
    pub cpuset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CpuTune {
    pub vcpupin: Vec<VcpuPin>,
    pub emulatorpin: Option<String>,
    pub iothreadpin: Vec<IoThreadPin>,
    pub shares: Option<u32>,
    pub period_us: Option<u32>,
    pub quota_us: Option<i32>,
    pub emulator_period_us: Option<u32>,
    pub emulator_quota_us: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VcpuPin {
    pub vcpu: u32,
    pub cpuset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct IoThreadPin {
    pub iothread: u32,
    pub cpuset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MemTune {
    pub hard_limit_kib: Option<u64>,
    pub soft_limit_kib: Option<u64>,
    pub swap_hard_limit_kib: Option<u64>,
    pub min_guarantee_kib: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Numa {
    pub cells: Vec<NumaCell>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NumaCell {
    pub id: u32,
    pub cpus: String,
    pub memory_kib: u64,
    pub memory_unit: String,
    pub distances: Vec<NumaDistance>,
    pub memory_access: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NumaDistance {
    pub cell_id: u32,
    pub value: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Hugepages {
    pub pages: Vec<HugePage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HugePage {
    pub size_kib: u64,
    pub nodeset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct IoThreadsConfig {
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CpuTuneSnapshot {
    pub cpu: CpuConfig,
    pub vcpus: VcpuConfig,
    pub cputune: CpuTune,
    pub memtune: MemTune,
    pub numa: Numa,
    pub hugepages: Hugepages,
    pub iothreads: IoThreadsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuTunePatch {
    pub cpu: Option<CpuConfig>,
    pub vcpus: Option<VcpuConfig>,
    pub cputune: Option<CpuTune>,
    pub memtune: Option<MemTune>,
    pub numa: Option<Numa>,
    pub hugepages: Option<Hugepages>,
    pub iothreads: Option<IoThreadsConfig>,
}

// ═════════════════════════════════════════════════════════════════════
// Parse
// ═════════════════════════════════════════════════════════════════════

pub fn parse(xml: &str) -> Result<CpuTuneSnapshot, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut snap = CpuTuneSnapshot::default();
    let mut path: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    let mut capture: Option<TextTarget> = None;
    let mut text_accum = String::new();
    let mut current_cell: Option<NumaCell> = None;

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
                handle_start(&n, &a, &path, &mut snap, &mut capture, &mut current_cell);
                path.push(n);
                text_accum.clear();
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                handle_empty(&n, &a, &path, &mut snap, &mut current_cell);
                capture = None;
            }
            Ok(Event::End(e)) => {
                let n = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if let Some(target) = capture {
                    let s = text_accum.trim().to_string();
                    apply_text(target, &s, &mut snap);
                    capture = None;
                }
                if n == "cell" {
                    if let Some(c) = current_cell.take() {
                        snap.numa.cells.push(c);
                    }
                }
                path.pop();
                text_accum.clear();
            }
            Ok(Event::Text(t)) => {
                let s = t.unescape().unwrap_or_default().to_string();
                text_accum.push_str(&s);
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(snap)
}

#[derive(Clone, Copy)]
enum TextTarget {
    VcpuMax,
    IoThreads,
    CpuTuneShares,
    CpuTunePeriod,
    CpuTuneQuota,
    CpuTuneEmulatorPeriod,
    CpuTuneEmulatorQuota,
    MemHardLimit,
    MemSoftLimit,
    MemSwapHardLimit,
    MemMinGuarantee,
    CpuModel,
}

fn handle_start(
    n: &str,
    a: &[(String, String)],
    path: &[String],
    snap: &mut CpuTuneSnapshot,
    capture: &mut Option<TextTarget>,
    current_cell: &mut Option<NumaCell>,
) {
    let parent = path.last().map(String::as_str);
    let attr = |k: &str| a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    match (parent, n) {
        (Some("domain"), "cpu") => {
            snap.cpu.mode = attr("mode").unwrap_or_default();
            snap.cpu.check = attr("check");
            snap.cpu.migratable = match attr("migratable").as_deref() {
                Some("on") => Some(true),
                Some("off") => Some(false),
                _ => None,
            };
        }
        (Some("cpu"), "model") => {
            *capture = Some(TextTarget::CpuModel);
        }
        (Some("cpu"), "numa") => {}
        (Some("numa"), "cell") => {
            let id = attr("id").and_then(|s| s.parse().ok()).unwrap_or(0);
            let cpus = attr("cpus").unwrap_or_default();
            let memory_kib = parse_memory_kib(attr("memory").as_deref(), attr("unit").as_deref());
            let memory_unit = attr("unit").unwrap_or_else(|| "KiB".to_string());
            let memory_access = attr("memAccess");
            *current_cell = Some(NumaCell {
                id,
                cpus,
                memory_kib,
                memory_unit,
                distances: Vec::new(),
                memory_access,
            });
        }
        (Some("cell"), "distances") => {}
        (Some("domain"), "vcpu") => {
            snap.vcpus.placement = attr("placement");
            snap.vcpus.cpuset = attr("cpuset");
            if let Some(c) = attr("current").and_then(|s| s.parse().ok()) {
                snap.vcpus.current = c;
            }
            *capture = Some(TextTarget::VcpuMax);
        }
        (Some("domain"), "iothreads") => {
            *capture = Some(TextTarget::IoThreads);
        }
        (Some("domain"), "cputune") => {}
        (Some("cputune"), "shares") => *capture = Some(TextTarget::CpuTuneShares),
        (Some("cputune"), "period") => *capture = Some(TextTarget::CpuTunePeriod),
        (Some("cputune"), "quota") => *capture = Some(TextTarget::CpuTuneQuota),
        (Some("cputune"), "emulator_period") => *capture = Some(TextTarget::CpuTuneEmulatorPeriod),
        (Some("cputune"), "emulator_quota") => *capture = Some(TextTarget::CpuTuneEmulatorQuota),
        (Some("domain"), "memtune") => {}
        (Some("memtune"), "hard_limit") => *capture = Some(TextTarget::MemHardLimit),
        (Some("memtune"), "soft_limit") => *capture = Some(TextTarget::MemSoftLimit),
        (Some("memtune"), "swap_hard_limit") => *capture = Some(TextTarget::MemSwapHardLimit),
        (Some("memtune"), "min_guarantee") => *capture = Some(TextTarget::MemMinGuarantee),
        (Some("domain"), "memoryBacking") => {}
        (Some("memoryBacking"), "hugepages") => {}
        _ => {}
    }
}

fn handle_empty(
    n: &str,
    a: &[(String, String)],
    path: &[String],
    snap: &mut CpuTuneSnapshot,
    current_cell: &mut Option<NumaCell>,
) {
    let parent = path.last().map(String::as_str);
    let attr = |k: &str| a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    match (parent, n) {
        (Some("domain"), "cpu") => {
            snap.cpu.mode = attr("mode").unwrap_or_default();
            snap.cpu.check = attr("check");
            snap.cpu.migratable = match attr("migratable").as_deref() {
                Some("on") => Some(true),
                Some("off") => Some(false),
                _ => None,
            };
        }
        (Some("domain"), "vcpu") => {
            snap.vcpus.placement = attr("placement");
            snap.vcpus.cpuset = attr("cpuset");
            if let Some(c) = attr("current").and_then(|s| s.parse().ok()) {
                snap.vcpus.current = c;
            }
        }
        (Some("cpu"), "topology") => {
            snap.cpu.topology = Some(CpuTopology {
                sockets: attr("sockets").and_then(|s| s.parse().ok()).unwrap_or(1),
                dies: attr("dies").and_then(|s| s.parse().ok()).unwrap_or(1),
                cores: attr("cores").and_then(|s| s.parse().ok()).unwrap_or(1),
                threads: attr("threads").and_then(|s| s.parse().ok()).unwrap_or(1),
            });
        }
        (Some("cpu"), "cache") => {
            snap.cpu.cache = Some(CpuCache {
                mode: attr("mode").unwrap_or_default(),
                level: attr("level").and_then(|s| s.parse().ok()),
            });
        }
        (Some("cpu"), "feature") => {
            snap.cpu.features.push(CpuFeature {
                name: attr("name").unwrap_or_default(),
                policy: attr("policy").unwrap_or_else(|| "require".into()),
            });
        }
        (Some("numa"), "cell") => {
            let id = attr("id").and_then(|s| s.parse().ok()).unwrap_or(0);
            let cpus = attr("cpus").unwrap_or_default();
            let memory_kib = parse_memory_kib(attr("memory").as_deref(), attr("unit").as_deref());
            let memory_unit = attr("unit").unwrap_or_else(|| "KiB".to_string());
            let memory_access = attr("memAccess");
            snap.numa.cells.push(NumaCell {
                id,
                cpus,
                memory_kib,
                memory_unit,
                distances: Vec::new(),
                memory_access,
            });
        }
        (Some("distances"), "sibling") => {
            if let Some(cell) = current_cell {
                cell.distances.push(NumaDistance {
                    cell_id: attr("id").and_then(|s| s.parse().ok()).unwrap_or(0),
                    value: attr("value").and_then(|s| s.parse().ok()).unwrap_or(10),
                });
            }
        }
        (Some("cputune"), "vcpupin") => {
            snap.cputune.vcpupin.push(VcpuPin {
                vcpu: attr("vcpu").and_then(|s| s.parse().ok()).unwrap_or(0),
                cpuset: attr("cpuset").unwrap_or_default(),
            });
        }
        (Some("cputune"), "emulatorpin") => {
            snap.cputune.emulatorpin = attr("cpuset");
        }
        (Some("cputune"), "iothreadpin") => {
            snap.cputune.iothreadpin.push(IoThreadPin {
                iothread: attr("iothread").and_then(|s| s.parse().ok()).unwrap_or(0),
                cpuset: attr("cpuset").unwrap_or_default(),
            });
        }
        (Some("hugepages"), "page") => {
            snap.hugepages.pages.push(HugePage {
                size_kib: parse_memory_kib(attr("size").as_deref(), attr("unit").as_deref()),
                nodeset: attr("nodeset"),
            });
        }
        _ => {}
    }
}

fn apply_text(target: TextTarget, s: &str, snap: &mut CpuTuneSnapshot) {
    match target {
        TextTarget::VcpuMax => {
            if let Ok(n) = s.parse() {
                snap.vcpus.max = n;
                if snap.vcpus.current == 0 {
                    snap.vcpus.current = n;
                }
            }
        }
        TextTarget::IoThreads => {
            if let Ok(n) = s.parse() { snap.iothreads.count = n; }
        }
        TextTarget::CpuTuneShares => {
            if let Ok(n) = s.parse() { snap.cputune.shares = Some(n); }
        }
        TextTarget::CpuTunePeriod => {
            if let Ok(n) = s.parse() { snap.cputune.period_us = Some(n); }
        }
        TextTarget::CpuTuneQuota => {
            if let Ok(n) = s.parse::<i32>() { snap.cputune.quota_us = Some(n); }
        }
        TextTarget::CpuTuneEmulatorPeriod => {
            if let Ok(n) = s.parse() { snap.cputune.emulator_period_us = Some(n); }
        }
        TextTarget::CpuTuneEmulatorQuota => {
            if let Ok(n) = s.parse::<i32>() { snap.cputune.emulator_quota_us = Some(n); }
        }
        TextTarget::MemHardLimit => {
            if let Ok(n) = s.parse::<u64>() { snap.memtune.hard_limit_kib = Some(n); }
        }
        TextTarget::MemSoftLimit => {
            if let Ok(n) = s.parse::<u64>() { snap.memtune.soft_limit_kib = Some(n); }
        }
        TextTarget::MemSwapHardLimit => {
            if let Ok(n) = s.parse::<u64>() { snap.memtune.swap_hard_limit_kib = Some(n); }
        }
        TextTarget::MemMinGuarantee => {
            if let Ok(n) = s.parse::<u64>() { snap.memtune.min_guarantee_kib = Some(n); }
        }
        TextTarget::CpuModel => {
            if !s.is_empty() {
                snap.cpu.model = Some(s.to_string());
            }
        }
    }
}

fn parse_memory_kib(val: Option<&str>, unit: Option<&str>) -> u64 {
    let raw: u64 = val.and_then(|s| s.parse().ok()).unwrap_or(0);
    match unit.map(str::to_ascii_lowercase).as_deref() {
        Some("mib") | Some("m") => raw * 1024,
        Some("gib") | Some("g") => raw * 1024 * 1024,
        Some("tib") | Some("t") => raw * 1024 * 1024 * 1024,
        Some("bytes") | Some("b") => raw / 1024,
        _ => raw,
    }
}

fn utf8_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes().filter_map(|a| a.ok()).map(|a| (
        String::from_utf8_lossy(a.key.as_ref()).to_string(),
        a.unescape_value().unwrap_or_default().to_string(),
    )).collect()
}

// ═════════════════════════════════════════════════════════════════════
// Validate
// ═════════════════════════════════════════════════════════════════════

pub fn validate(snap: &CpuTuneSnapshot) -> Result<(), VirtManagerError> {
    if snap.vcpus.max > 0 && snap.vcpus.current > snap.vcpus.max {
        return Err(VirtManagerError::OperationFailed { operation: "validate".into(), reason: format!(
                "vcpu.current ({}) must be <= vcpu.max ({})",
                snap.vcpus.current, snap.vcpus.max
            ),
        });
    }
    if let (Some(h), Some(s)) = (snap.memtune.hard_limit_kib, snap.memtune.soft_limit_kib) {
        if h < s {
            return Err(VirtManagerError::OperationFailed { operation: "validate".into(), reason: format!("hard_limit ({}) must be >= soft_limit ({})", h, s),
            });
        }
    }
    if let (Some(h), Some(swap)) = (snap.memtune.hard_limit_kib, snap.memtune.swap_hard_limit_kib) {
        if swap < h {
            return Err(VirtManagerError::OperationFailed { operation: "validate".into(), reason: format!("swap_hard_limit ({}) must be >= hard_limit ({})", swap, h),
            });
        }
    }
    if snap.cpu.migratable == Some(true) && !snap.cpu.mode.is_empty()
        && snap.cpu.mode != "host-passthrough"
    {
        return Err(VirtManagerError::OperationFailed { operation: "validate".into(), reason: format!(
                "migratable='on' is only valid for mode='host-passthrough' (got '{}')",
                snap.cpu.mode
            ),
        });
    }
    for f in &snap.cpu.features {
        match f.policy.as_str() {
            "force" | "require" | "optional" | "disable" | "forbid" => {}
            other => {
                return Err(VirtManagerError::OperationFailed { operation: "validate".into(), reason: format!("unknown feature policy '{}'", other),
                });
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for c in &snap.numa.cells {
        if !seen.insert(c.id) {
            return Err(VirtManagerError::OperationFailed { operation: "validate".into(), reason: format!("duplicate NUMA cell id {}", c.id),
            });
        }
    }
    Ok(())
}

// ═════════════════════════════════════════════════════════════════════
// Apply
// ═════════════════════════════════════════════════════════════════════

pub fn apply(xml: &str, patch: &CpuTunePatch) -> Result<String, VirtManagerError> {
    let mut out = xml.to_string();

    if let Some(ref cpu) = patch.cpu {
        let block = build_cpu_block_with_numa(cpu, patch.numa.as_ref());
        out = replace_element_block(&out, "cpu", &block)?;
    } else if patch.numa.is_some() {
        let current = parse(&out)?;
        let block = build_cpu_block_with_numa(&current.cpu, patch.numa.as_ref());
        out = replace_element_block(&out, "cpu", &block)?;
    }
    if let Some(ref v) = patch.vcpus {
        let block = build_vcpu_block(v);
        out = replace_element_block(&out, "vcpu", &block)?;
    }
    if let Some(ref ct) = patch.cputune {
        let block = build_cputune_block(ct);
        out = replace_element_block(&out, "cputune", &block)?;
    }
    if let Some(ref mt) = patch.memtune {
        let block = build_memtune_block(mt);
        out = replace_element_block(&out, "memtune", &block)?;
    }
    if let Some(ref hp) = patch.hugepages {
        let block = build_memory_backing_block(hp);
        out = replace_element_block(&out, "memoryBacking", &block)?;
    }
    if let Some(ref it) = patch.iothreads {
        if it.count == 0 {
            out = remove_element(&out, "iothreads");
        } else {
            let block = format!("<iothreads>{}</iothreads>", it.count);
            out = replace_element_block(&out, "iothreads", &block)?;
        }
    }
    Ok(out)
}

fn build_cpu_block_with_numa(cpu: &CpuConfig, numa: Option<&Numa>) -> String {
    let mut s = String::from("<cpu");
    if !cpu.mode.is_empty() {
        s.push_str(&format!(" mode='{}'", escape_xml(&cpu.mode)));
    }
    if let Some(ref c) = cpu.check {
        s.push_str(&format!(" check='{}'", escape_xml(c)));
    }
    if cpu.migratable.is_some() {
        let v = if cpu.migratable == Some(true) { "on" } else { "off" };
        s.push_str(&format!(" migratable='{}'", v));
    }
    let has_numa = numa.map(|n| !n.cells.is_empty()).unwrap_or(false);
    let has_children = cpu.model.is_some()
        || cpu.topology.is_some()
        || !cpu.features.is_empty()
        || cpu.cache.is_some()
        || has_numa;
    if !has_children {
        s.push_str("/>");
        return s;
    }
    s.push_str(">\n");
    if let Some(ref m) = cpu.model {
        s.push_str(&format!("    <model>{}</model>\n", escape_xml(m)));
    }
    if let Some(ref t) = cpu.topology {
        s.push_str(&format!(
            "    <topology sockets='{}' dies='{}' cores='{}' threads='{}'/>\n",
            t.sockets, t.dies, t.cores, t.threads
        ));
    }
    for f in &cpu.features {
        s.push_str(&format!(
            "    <feature policy='{}' name='{}'/>\n",
            escape_xml(&f.policy), escape_xml(&f.name)
        ));
    }
    if let Some(ref c) = cpu.cache {
        match c.level {
            Some(l) => s.push_str(&format!("    <cache level='{}' mode='{}'/>\n", l, escape_xml(&c.mode))),
            None => s.push_str(&format!("    <cache mode='{}'/>\n", escape_xml(&c.mode))),
        }
    }
    if let Some(n) = numa {
        if !n.cells.is_empty() {
            s.push_str("    <numa>\n");
            for cell in &n.cells {
                s.push_str(&format!("      <cell id='{}' cpus='{}' memory='{}' unit='{}'",
                    cell.id, escape_xml(&cell.cpus), cell.memory_kib,
                    escape_xml(&cell.memory_unit)));
                if let Some(ref ma) = cell.memory_access {
                    s.push_str(&format!(" memAccess='{}'", escape_xml(ma)));
                }
                if cell.distances.is_empty() {
                    s.push_str("/>\n");
                } else {
                    s.push_str(">\n");
                    s.push_str("        <distances>\n");
                    for d in &cell.distances {
                        s.push_str(&format!(
                            "          <sibling id='{}' value='{}'/>\n",
                            d.cell_id, d.value
                        ));
                    }
                    s.push_str("        </distances>\n");
                    s.push_str("      </cell>\n");
                }
            }
            s.push_str("    </numa>\n");
        }
    }
    s.push_str("  </cpu>");
    s
}

fn build_vcpu_block(v: &VcpuConfig) -> String {
    let mut attrs_s = String::new();
    if let Some(ref p) = v.placement {
        attrs_s.push_str(&format!(" placement='{}'", escape_xml(p)));
    }
    if v.current > 0 && v.current != v.max {
        attrs_s.push_str(&format!(" current='{}'", v.current));
    }
    if let Some(ref c) = v.cpuset {
        if !c.is_empty() {
            attrs_s.push_str(&format!(" cpuset='{}'", escape_xml(c)));
        }
    }
    format!("<vcpu{}>{}</vcpu>", attrs_s, v.max.max(1))
}

fn build_cputune_block(ct: &CpuTune) -> String {
    let mut s = String::from("<cputune>\n");
    for p in &ct.vcpupin {
        s.push_str(&format!(
            "    <vcpupin vcpu='{}' cpuset='{}'/>\n",
            p.vcpu, escape_xml(&p.cpuset)
        ));
    }
    if let Some(ref e) = ct.emulatorpin {
        s.push_str(&format!("    <emulatorpin cpuset='{}'/>\n", escape_xml(e)));
    }
    for p in &ct.iothreadpin {
        s.push_str(&format!(
            "    <iothreadpin iothread='{}' cpuset='{}'/>\n",
            p.iothread, escape_xml(&p.cpuset)
        ));
    }
    if let Some(v) = ct.shares { s.push_str(&format!("    <shares>{}</shares>\n", v)); }
    if let Some(v) = ct.period_us { s.push_str(&format!("    <period>{}</period>\n", v)); }
    if let Some(v) = ct.quota_us { s.push_str(&format!("    <quota>{}</quota>\n", v)); }
    if let Some(v) = ct.emulator_period_us {
        s.push_str(&format!("    <emulator_period>{}</emulator_period>\n", v));
    }
    if let Some(v) = ct.emulator_quota_us {
        s.push_str(&format!("    <emulator_quota>{}</emulator_quota>\n", v));
    }
    s.push_str("  </cputune>");
    s
}

fn build_memtune_block(mt: &MemTune) -> String {
    let mut s = String::from("<memtune>\n");
    if let Some(v) = mt.hard_limit_kib {
        s.push_str(&format!("    <hard_limit unit='KiB'>{}</hard_limit>\n", v));
    }
    if let Some(v) = mt.soft_limit_kib {
        s.push_str(&format!("    <soft_limit unit='KiB'>{}</soft_limit>\n", v));
    }
    if let Some(v) = mt.swap_hard_limit_kib {
        s.push_str(&format!("    <swap_hard_limit unit='KiB'>{}</swap_hard_limit>\n", v));
    }
    if let Some(v) = mt.min_guarantee_kib {
        s.push_str(&format!("    <min_guarantee unit='KiB'>{}</min_guarantee>\n", v));
    }
    s.push_str("  </memtune>");
    s
}

fn build_memory_backing_block(hp: &Hugepages) -> String {
    let mut s = String::from("<memoryBacking>\n");
    if !hp.pages.is_empty() {
        s.push_str("    <hugepages>\n");
        for p in &hp.pages {
            s.push_str(&format!("      <page size='{}' unit='KiB'", p.size_kib));
            if let Some(ref n) = p.nodeset {
                s.push_str(&format!(" nodeset='{}'", escape_xml(n)));
            }
            s.push_str("/>\n");
        }
        s.push_str("    </hugepages>\n");
    }
    s.push_str("  </memoryBacking>");
    s
}

fn replace_element_block(
    xml: &str,
    name: &str,
    new_content: &str,
) -> Result<String, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
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
                if depth == 0 { start_byte = Some(pos_before); }
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
        _ => inject(xml, new_content),
    }
}

fn inject(xml: &str, new_content: &str) -> Result<String, VirtManagerError> {
    if let Some(idx) = xml.find("<devices>") {
        let mut out = String::with_capacity(xml.len() + new_content.len() + 4);
        out.push_str(&xml[..idx]);
        out.push_str(new_content);
        out.push('\n');
        out.push_str("  ");
        out.push_str(&xml[idx..]);
        Ok(out)
    } else if let Some(idx) = xml.rfind("</domain>") {
        let mut out = String::with_capacity(xml.len() + new_content.len() + 4);
        out.push_str(&xml[..idx]);
        out.push_str("  ");
        out.push_str(new_content);
        out.push('\n');
        out.push_str(&xml[idx..]);
        Ok(out)
    } else {
        Err(VirtManagerError::XmlParsingFailed {
            reason: "could not find insertion point (no <devices> or </domain>)".into(),
        })
    }
}

fn remove_element(xml: &str, name: &str) -> String {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth: i32 = 0;
    let mut start_byte: Option<usize> = None;
    let mut end_byte: Option<usize> = None;

    loop {
        let pos_before = r.buffer_position() as usize;
        match r.read_event_into(&mut buf) {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if String::from_utf8_lossy(e.name().as_ref()) == name => {
                if depth == 0 { start_byte = Some(pos_before); }
                depth += 1;
            }
            Ok(Event::Empty(e)) if String::from_utf8_lossy(e.name().as_ref()) == name => {
                start_byte = Some(pos_before);
                end_byte = Some(r.buffer_position() as usize);
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
            let mut out = String::with_capacity(xml.len());
            out.push_str(&xml[..s]);
            if !out.ends_with('\n') { out.push('\n'); }
            out.push_str("  ");
            let remaining = xml[e..].trim_start_matches(|c: char| c.is_whitespace());
            out.push_str(remaining);
            out
        }
        _ => xml.to_string(),
    }
}

// ═════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"<domain type='kvm'>
  <name>t</name>
  <memory unit='KiB'>8388608</memory>
  <vcpu placement='static' current='2' cpuset='0-3'>4</vcpu>
  <iothreads>4</iothreads>
  <cputune>
    <vcpupin vcpu='0' cpuset='0'/>
    <vcpupin vcpu='1' cpuset='1'/>
    <emulatorpin cpuset='2-3'/>
    <iothreadpin iothread='1' cpuset='4'/>
    <shares>2048</shares>
    <period>100000</period>
    <quota>50000</quota>
    <emulator_period>100000</emulator_period>
    <emulator_quota>10000</emulator_quota>
  </cputune>
  <memtune>
    <hard_limit unit='KiB'>4194304</hard_limit>
    <soft_limit unit='KiB'>2097152</soft_limit>
    <swap_hard_limit unit='KiB'>4194304</swap_hard_limit>
    <min_guarantee unit='KiB'>1048576</min_guarantee>
  </memtune>
  <memoryBacking>
    <hugepages>
      <page size='2048' unit='KiB' nodeset='0'/>
      <page size='1048576' unit='KiB' nodeset='1'/>
    </hugepages>
  </memoryBacking>
  <os>
    <type arch='x86_64' machine='q35'>hvm</type>
  </os>
  <cpu mode='custom' check='partial'>
    <model>Skylake-Server</model>
    <topology sockets='2' dies='1' cores='4' threads='2'/>
    <feature policy='require' name='vmx'/>
    <feature policy='disable' name='hypervisor'/>
    <cache mode='passthrough'/>
    <numa>
      <cell id='0' cpus='0-3' memory='4194304' unit='KiB' memAccess='shared'>
        <distances>
          <sibling id='0' value='10'/>
          <sibling id='1' value='20'/>
        </distances>
      </cell>
      <cell id='1' cpus='4-7' memory='4194304' unit='KiB'/>
    </numa>
  </cpu>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
  </devices>
</domain>
"#;

    const MINIMAL: &str = r#"<domain>
  <vcpu>1</vcpu>
  <cpu mode='host-passthrough' check='none' migratable='on'/>
  <devices/>
</domain>
"#;

    #[test]
    fn parse_full_topology() {
        let s = parse(FULL).unwrap();
        assert_eq!(s.cpu.mode, "custom");
        assert_eq!(s.cpu.check.as_deref(), Some("partial"));
        assert_eq!(s.cpu.model.as_deref(), Some("Skylake-Server"));
        let t = s.cpu.topology.unwrap();
        assert_eq!((t.sockets, t.dies, t.cores, t.threads), (2, 1, 4, 2));
        assert_eq!(s.cpu.features.len(), 2);
        assert_eq!(s.cpu.features[0].name, "vmx");
        assert_eq!(s.cpu.features[0].policy, "require");
        assert_eq!(s.cpu.cache.as_ref().unwrap().mode, "passthrough");
    }

    #[test]
    fn parse_minimal_cpu_mode_only() {
        let s = parse(MINIMAL).unwrap();
        assert_eq!(s.cpu.mode, "host-passthrough");
        assert_eq!(s.cpu.migratable, Some(true));
        assert_eq!(s.vcpus.max, 1);
        assert_eq!(s.vcpus.current, 1);
        assert!(s.cputune.vcpupin.is_empty());
        assert!(s.memtune.hard_limit_kib.is_none());
    }

    #[test]
    fn parse_vcpu_with_current_max_cpuset() {
        let s = parse(FULL).unwrap();
        assert_eq!(s.vcpus.max, 4);
        assert_eq!(s.vcpus.current, 2);
        assert_eq!(s.vcpus.placement.as_deref(), Some("static"));
        assert_eq!(s.vcpus.cpuset.as_deref(), Some("0-3"));
    }

    #[test]
    fn parse_cputune_full() {
        let s = parse(FULL).unwrap();
        let ct = &s.cputune;
        assert_eq!(ct.vcpupin.len(), 2);
        assert_eq!(ct.vcpupin[0].vcpu, 0);
        assert_eq!(ct.vcpupin[0].cpuset, "0");
        assert_eq!(ct.emulatorpin.as_deref(), Some("2-3"));
        assert_eq!(ct.iothreadpin.len(), 1);
        assert_eq!(ct.iothreadpin[0].iothread, 1);
        assert_eq!(ct.shares, Some(2048));
        assert_eq!(ct.period_us, Some(100_000));
        assert_eq!(ct.quota_us, Some(50_000));
        assert_eq!(ct.emulator_period_us, Some(100_000));
        assert_eq!(ct.emulator_quota_us, Some(10_000));
    }

    #[test]
    fn parse_memtune_full() {
        let s = parse(FULL).unwrap();
        assert_eq!(s.memtune.hard_limit_kib, Some(4_194_304));
        assert_eq!(s.memtune.soft_limit_kib, Some(2_097_152));
        assert_eq!(s.memtune.swap_hard_limit_kib, Some(4_194_304));
        assert_eq!(s.memtune.min_guarantee_kib, Some(1_048_576));
    }

    #[test]
    fn parse_numa_with_distances() {
        let s = parse(FULL).unwrap();
        assert_eq!(s.numa.cells.len(), 2);
        let c0 = &s.numa.cells[0];
        assert_eq!(c0.id, 0);
        assert_eq!(c0.cpus, "0-3");
        assert_eq!(c0.memory_kib, 4_194_304);
        assert_eq!(c0.memory_access.as_deref(), Some("shared"));
        assert_eq!(c0.distances.len(), 2);
        assert_eq!(c0.distances[0].cell_id, 0);
        assert_eq!(c0.distances[0].value, 10);
        assert_eq!(c0.distances[1].value, 20);
        let c1 = &s.numa.cells[1];
        assert!(c1.distances.is_empty());
        assert!(c1.memory_access.is_none());
    }

    #[test]
    fn parse_hugepages_multi_page() {
        let s = parse(FULL).unwrap();
        assert_eq!(s.hugepages.pages.len(), 2);
        assert_eq!(s.hugepages.pages[0].size_kib, 2048);
        assert_eq!(s.hugepages.pages[0].nodeset.as_deref(), Some("0"));
        assert_eq!(s.hugepages.pages[1].size_kib, 1_048_576);
        assert_eq!(s.hugepages.pages[1].nodeset.as_deref(), Some("1"));
    }

    #[test]
    fn parse_iothreads_count() {
        let s = parse(FULL).unwrap();
        assert_eq!(s.iothreads.count, 4);
        let s2 = parse(MINIMAL).unwrap();
        assert_eq!(s2.iothreads.count, 0);
    }

    #[test]
    fn roundtrip_cpu_topology() {
        let s = parse(FULL).unwrap();
        let patch = CpuTunePatch { cpu: Some(s.cpu.clone()), ..Default::default() };
        let out = apply(FULL, &patch).unwrap();
        let s2 = parse(&out).unwrap();
        assert_eq!(s.cpu.topology, s2.cpu.topology);
        assert_eq!(s.cpu.mode, s2.cpu.mode);
        assert_eq!(s.cpu.model, s2.cpu.model);
    }

    #[test]
    fn roundtrip_cpu_features_all_policies() {
        let mut cpu = CpuConfig { mode: "custom".into(), ..Default::default() };
        for pol in ["force", "require", "optional", "disable", "forbid"] {
            cpu.features.push(CpuFeature { name: format!("feat_{}", pol), policy: pol.into() });
        }
        let patch = CpuTunePatch { cpu: Some(cpu.clone()), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        let s = parse(&out).unwrap();
        assert_eq!(s.cpu.features.len(), 5);
        for (expected, got) in cpu.features.iter().zip(s.cpu.features.iter()) {
            assert_eq!(expected.name, got.name);
            assert_eq!(expected.policy, got.policy);
        }
    }

    #[test]
    fn roundtrip_vcpu_config() {
        let v = VcpuConfig {
            max: 8, current: 4,
            placement: Some("static".into()),
            cpuset: Some("0-7".into()),
        };
        let patch = CpuTunePatch { vcpus: Some(v.clone()), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        let s = parse(&out).unwrap();
        assert_eq!(s.vcpus.max, 8);
        assert_eq!(s.vcpus.current, 4);
        assert_eq!(s.vcpus.cpuset.as_deref(), Some("0-7"));
    }

    #[test]
    fn roundtrip_cputune_full() {
        let ct = CpuTune {
            vcpupin: vec![VcpuPin { vcpu: 0, cpuset: "0".into() }, VcpuPin { vcpu: 1, cpuset: "1".into() }],
            emulatorpin: Some("2-3".into()),
            iothreadpin: vec![IoThreadPin { iothread: 1, cpuset: "4".into() }],
            shares: Some(1024),
            period_us: Some(100_000),
            quota_us: Some(50_000),
            emulator_period_us: Some(100_000),
            emulator_quota_us: Some(10_000),
        };
        let patch = CpuTunePatch { cputune: Some(ct.clone()), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        let s = parse(&out).unwrap();
        assert_eq!(s.cputune, ct);
    }

    #[test]
    fn roundtrip_memtune() {
        let mt = MemTune {
            hard_limit_kib: Some(4_000_000),
            soft_limit_kib: Some(2_000_000),
            swap_hard_limit_kib: Some(5_000_000),
            min_guarantee_kib: Some(1_000_000),
        };
        let patch = CpuTunePatch { memtune: Some(mt.clone()), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        let s = parse(&out).unwrap();
        assert_eq!(s.memtune, mt);
    }

    #[test]
    fn roundtrip_numa() {
        let numa = Numa {
            cells: vec![
                NumaCell {
                    id: 0, cpus: "0-1".into(), memory_kib: 2_097_152,
                    memory_unit: "KiB".into(),
                    distances: vec![
                        NumaDistance { cell_id: 0, value: 10 },
                        NumaDistance { cell_id: 1, value: 20 },
                    ],
                    memory_access: Some("shared".into()),
                },
                NumaCell {
                    id: 1, cpus: "2-3".into(), memory_kib: 2_097_152,
                    memory_unit: "KiB".into(),
                    distances: vec![],
                    memory_access: None,
                },
            ],
        };
        let patch = CpuTunePatch { numa: Some(numa.clone()), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        let s = parse(&out).unwrap();
        assert_eq!(s.numa.cells.len(), 2);
        assert_eq!(s.numa.cells[0].distances.len(), 2);
        assert_eq!(s.numa.cells[0].memory_access.as_deref(), Some("shared"));
    }

    #[test]
    fn roundtrip_hugepages() {
        let hp = Hugepages {
            pages: vec![
                HugePage { size_kib: 2048, nodeset: Some("0".into()) },
                HugePage { size_kib: 1_048_576, nodeset: None },
            ],
        };
        let patch = CpuTunePatch { hugepages: Some(hp.clone()), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        let s = parse(&out).unwrap();
        assert_eq!(s.hugepages.pages.len(), 2);
        assert_eq!(s.hugepages.pages[0].size_kib, 2048);
        assert_eq!(s.hugepages.pages[1].nodeset, None);
    }

    #[test]
    fn roundtrip_iothreads_count() {
        let it = IoThreadsConfig { count: 6 };
        let patch = CpuTunePatch { iothreads: Some(it), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        let s = parse(&out).unwrap();
        assert_eq!(s.iothreads.count, 6);

        let patch2 = CpuTunePatch { iothreads: Some(IoThreadsConfig { count: 0 }), ..Default::default() };
        let out2 = apply(&out, &patch2).unwrap();
        let s2 = parse(&out2).unwrap();
        assert_eq!(s2.iothreads.count, 0);
        assert!(!out2.contains("<iothreads>"));
    }

    #[test]
    fn validate_rejects_vcpu_current_over_max() {
        let mut s = CpuTuneSnapshot::default();
        s.vcpus.max = 2;
        s.vcpus.current = 4;
        assert!(validate(&s).is_err());
    }

    #[test]
    fn validate_rejects_hard_below_soft() {
        let mut s = CpuTuneSnapshot::default();
        s.memtune.hard_limit_kib = Some(1_000);
        s.memtune.soft_limit_kib = Some(2_000);
        assert!(validate(&s).is_err());
    }

    #[test]
    fn validate_rejects_migratable_without_passthrough() {
        let mut s = CpuTuneSnapshot::default();
        s.cpu.mode = "host-model".into();
        s.cpu.migratable = Some(true);
        assert!(validate(&s).is_err());
    }

    #[test]
    fn validate_rejects_unknown_feature_policy() {
        let mut s = CpuTuneSnapshot::default();
        s.cpu.features.push(CpuFeature { name: "x".into(), policy: "bogus".into() });
        assert!(validate(&s).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_numa_cell_id() {
        let mut s = CpuTuneSnapshot::default();
        s.numa.cells.push(NumaCell { id: 0, cpus: "0".into(), memory_kib: 1, memory_unit: "KiB".into(), distances: vec![], memory_access: None });
        s.numa.cells.push(NumaCell { id: 0, cpus: "1".into(), memory_kib: 1, memory_unit: "KiB".into(), distances: vec![], memory_access: None });
        assert!(validate(&s).is_err());
    }

    #[test]
    fn validate_accepts_sensible_config() {
        let mut s = CpuTuneSnapshot::default();
        s.vcpus.max = 4; s.vcpus.current = 2;
        s.cpu.mode = "host-passthrough".into();
        s.cpu.migratable = Some(true);
        s.memtune.hard_limit_kib = Some(2000);
        s.memtune.soft_limit_kib = Some(1000);
        s.memtune.swap_hard_limit_kib = Some(2000);
        assert!(validate(&s).is_ok());
    }

    #[test]
    fn apply_preserves_unrelated_sections() {
        let patch = CpuTunePatch {
            iothreads: Some(IoThreadsConfig { count: 2 }),
            ..Default::default()
        };
        let out = apply(FULL, &patch).unwrap();
        assert!(out.contains("<emulator>/usr/bin/qemu-system-x86_64</emulator>"));
        assert!(out.contains("<memory unit='KiB'>8388608</memory>"));
        assert!(out.contains("machine='q35'"));
        assert!(out.contains("<model>Skylake-Server</model>"));
    }

    #[test]
    fn apply_is_idempotent() {
        let patch = CpuTunePatch {
            cpu: Some(CpuConfig {
                mode: "host-model".into(),
                check: Some("partial".into()),
                ..Default::default()
            }),
            vcpus: Some(VcpuConfig {
                max: 4, current: 2,
                placement: Some("static".into()),
                cpuset: None,
            }),
            iothreads: Some(IoThreadsConfig { count: 2 }),
            ..Default::default()
        };
        let once = apply(MINIMAL, &patch).unwrap();
        let twice = apply(&once, &patch).unwrap();
        assert_eq!(parse(&once).unwrap(), parse(&twice).unwrap());
    }

    #[test]
    fn escape_xml_handles_injection_attempt() {
        let cpu = CpuConfig {
            mode: "custom".into(),
            model: Some("Model'><script>".into()),
            ..Default::default()
        };
        let patch = CpuTunePatch { cpu: Some(cpu), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        assert!(!out.contains("<script>"));
        assert!(out.contains("&lt;script&gt;"));
    }

    #[test]
    fn apply_cpu_cache_with_level() {
        let cpu = CpuConfig {
            mode: "host-passthrough".into(),
            cache: Some(CpuCache { mode: "passthrough".into(), level: Some(3) }),
            ..Default::default()
        };
        let patch = CpuTunePatch { cpu: Some(cpu), ..Default::default() };
        let out = apply(MINIMAL, &patch).unwrap();
        let s = parse(&out).unwrap();
        let c = s.cpu.cache.unwrap();
        assert_eq!(c.mode, "passthrough");
        assert_eq!(c.level, Some(3));
    }
}
