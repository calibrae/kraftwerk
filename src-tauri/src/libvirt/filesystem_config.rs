//! Filesystem passthrough + shared memory (`<filesystem>` and `<shmem>`).
//!
//! Covers the two host<->guest data-sharing devices grouped together in
//! this round. Both are additive (we add/remove whole device entries
//! rather than patching existing attributes), so the shape mirrors
//! `hostdev.rs`: parse -> Vec<T>, build_xml for single entries, and
//! apply_add/apply_remove that splice a fragment in/out of the
//! `<devices>` block.
//!
//! Driver matrix:
//! - `virtiofs` - the modern Linux-host / Linux-guest share. Requires
//!   `<memoryBacking><access mode='shared'/></memoryBacking>` because
//!   virtiofsd mmaps the guest RAM. Supports live hot-plug. Takes its
//!   own tuning flags: `queue_size`, `xattr`, `posix_lock`, `flock`,
//!   and an optional `<binary path=...>` override.
//! - `path` / `handle` - legacy 9p. The guest needs the 9p kernel
//!   module. Takes an `accessmode` (passthrough / mapped / squash).
//!   `multidevs` is 9p-specific.
//! - `loop` / `nbd` / `ploop` - exotic, built-in support but rarely
//!   used. We parse/serialise them but the UI wont expose them.
//!
//! `accessmode` is NOT valid on virtiofs. `queue_size` / `xattr` /
//! `posix_lock` / `flock` / `binary_path` are only meaningful on
//! virtiofs. The builder enforces this - the connection-level
//! add_filesystem returns a clear error if the caller tries to mix.

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

fn invalid(reason: impl Into<String>) -> VirtManagerError {
    VirtManagerError::OperationFailed {
        operation: "validate_filesystem".into(),
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilesystemDriver {
    Path,
    Handle,
    Loop,
    Nbd,
    Ploop,
    Virtiofs,
}

impl FilesystemDriver {
    pub fn as_str(&self) -> &'static str {
        match self {
            FilesystemDriver::Path => "path",
            FilesystemDriver::Handle => "handle",
            FilesystemDriver::Loop => "loop",
            FilesystemDriver::Nbd => "nbd",
            FilesystemDriver::Ploop => "ploop",
            FilesystemDriver::Virtiofs => "virtiofs",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "path" => Some(Self::Path),
            "handle" => Some(Self::Handle),
            "loop" => Some(Self::Loop),
            "nbd" => Some(Self::Nbd),
            "ploop" => Some(Self::Ploop),
            "virtiofs" => Some(Self::Virtiofs),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilesystemAccessMode {
    Passthrough,
    Mapped,
    Squash,
}

impl FilesystemAccessMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            FilesystemAccessMode::Passthrough => "passthrough",
            FilesystemAccessMode::Mapped => "mapped",
            FilesystemAccessMode::Squash => "squash",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "passthrough" => Some(Self::Passthrough),
            "mapped" => Some(Self::Mapped),
            "squash" => Some(Self::Squash),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MultidevsMode {
    Default,
    Remap,
    Forbid,
    Warn,
}

impl MultidevsMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            MultidevsMode::Default => "default",
            MultidevsMode::Remap => "remap",
            MultidevsMode::Forbid => "forbid",
            MultidevsMode::Warn => "warn",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::Default),
            "remap" => Some(Self::Remap),
            "forbid" => Some(Self::Forbid),
            "warn" => Some(Self::Warn),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemConfig {
    pub driver_type: FilesystemDriver,
    pub source_dir: String,
    pub target_dir: String,
    pub accessmode: Option<FilesystemAccessMode>,
    pub readonly: bool,
    pub multidevs: Option<MultidevsMode>,
    pub queue_size: Option<u32>,
    pub xattr: bool,
    pub posix_lock: bool,
    pub flock: bool,
    pub binary_path: Option<String>,
}

impl FilesystemConfig {
    pub fn virtiofs(source_dir: impl Into<String>, target_dir: impl Into<String>) -> Self {
        Self {
            driver_type: FilesystemDriver::Virtiofs,
            source_dir: source_dir.into(),
            target_dir: target_dir.into(),
            accessmode: None,
            readonly: false,
            multidevs: None,
            queue_size: None,
            xattr: false,
            posix_lock: false,
            flock: false,
            binary_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShmemConfig {
    pub name: String,
    pub size_bytes: u64,
    pub model: ShmemModel,
    pub role: ShmemRole,
    pub server: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShmemModel {
    IvshmemPlain,
    IvshmemDoorbell,
}

impl ShmemModel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShmemModel::IvshmemPlain => "ivshmem-plain",
            ShmemModel::IvshmemDoorbell => "ivshmem-doorbell",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ivshmem-plain" => Some(Self::IvshmemPlain),
            "ivshmem-doorbell" => Some(Self::IvshmemDoorbell),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShmemRole {
    Master,
    Peer,
}

impl ShmemRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShmemRole::Master => "master",
            ShmemRole::Peer => "peer",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "master" => Some(Self::Master),
            "peer" => Some(Self::Peer),
            _ => None,
        }
    }
}

pub fn validate_filesystem(fs: &FilesystemConfig) -> Result<(), VirtManagerError> {
    if fs.source_dir.is_empty() {
        return Err(invalid("filesystem source_dir must not be empty"));
    }
    if fs.target_dir.is_empty() {
        return Err(invalid("filesystem target_dir (mount tag) must not be empty"));
    }
    if fs.driver_type == FilesystemDriver::Virtiofs {
        // libvirt auto-normalises virtiofs to accessmode=passthrough when
        // it redefines the domain; tolerate that on parse round-trips but
        // reject an explicit mapped/squash which QEMU will refuse.
        if matches!(
            fs.accessmode,
            Some(FilesystemAccessMode::Mapped) | Some(FilesystemAccessMode::Squash)
        ) {
            return Err(invalid("accessmode is not valid for virtiofs driver"));
        }
    }
    let has_virtiofs_flag = fs.queue_size.is_some()
        || fs.xattr
        || fs.posix_lock
        || fs.flock
        || fs.binary_path.is_some();
    if has_virtiofs_flag && fs.driver_type != FilesystemDriver::Virtiofs {
        return Err(invalid(
            "queue_size/xattr/posix_lock/flock/binary_path are only valid for virtiofs",
        ));
    }
    Ok(())
}

pub fn parse_filesystems(xml: &str) -> Result<Vec<FilesystemConfig>, VirtManagerError> {
    let mut r = mk_reader(xml);
    let mut buf = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut out: Vec<FilesystemConfig> = Vec::new();

    let mut in_fs = false;
    let mut accessmode: Option<FilesystemAccessMode> = None;
    let mut multidevs: Option<MultidevsMode> = None;
    let mut driver_type: Option<FilesystemDriver> = None;
    let mut queue_size: Option<u32> = None;
    let mut source_dir = String::new();
    let mut target_dir = String::new();
    let mut readonly = false;
    let mut xattr = false;
    let mut posix_lock = false;
    let mut flock = false;
    let mut binary_path: Option<String> = None;

    let apply_child = |name: &str,
                       a: &[(String, String)],
                       driver_type: &mut Option<FilesystemDriver>,
                       queue_size: &mut Option<u32>,
                       source_dir: &mut String,
                       target_dir: &mut String,
                       xattr: &mut bool,
                       binary_path: &mut Option<String>| {
        match name {
            "driver" => {
                if let Some(t) = get_attr(a, "type") {
                    *driver_type = FilesystemDriver::parse(&t);
                }
                if let Some(q) = get_attr(a, "queue") {
                    *queue_size = q.parse().ok();
                }
            }
            "source" => {
                if let Some(d) = get_attr(a, "dir") {
                    *source_dir = d;
                }
            }
            "target" => {
                if let Some(d) = get_attr(a, "dir") {
                    *target_dir = d;
                }
            }
            "binary" => {
                if let Some(p) = get_attr(a, "path") {
                    *binary_path = Some(p);
                }
                if get_attr(a, "xattr").as_deref() == Some("on") {
                    *xattr = true;
                }
            }
            _ => {}
        }
    };

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(xml_err(e, r.buffer_position())),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if n == "filesystem" {
                    in_fs = true;
                    accessmode = get_attr(&a, "accessmode").and_then(|s| FilesystemAccessMode::parse(&s));
                    multidevs = get_attr(&a, "multidevs").and_then(|s| MultidevsMode::parse(&s));
                }
                if in_fs && path.last().map(String::as_str) == Some("filesystem") {
                    apply_child(&n, &a, &mut driver_type, &mut queue_size,
                                &mut source_dir, &mut target_dir, &mut xattr, &mut binary_path);
                }
                if in_fs
                    && path.len() >= 2
                    && path[path.len() - 1] == "binary"
                    && path[path.len() - 2] == "filesystem"
                    && n == "lock"
                {
                    if get_attr(&a, "posixlock").as_deref() == Some("on") || get_attr(&a, "posix").as_deref() == Some("on") {
                        posix_lock = true;
                    }
                    if get_attr(&a, "flock").as_deref() == Some("on") {
                        flock = true;
                    }
                }
                path.push(n);
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if in_fs && path.last().map(String::as_str) == Some("filesystem") {
                    apply_child(&n, &a, &mut driver_type, &mut queue_size,
                                &mut source_dir, &mut target_dir, &mut xattr, &mut binary_path);
                    if n == "readonly" {
                        readonly = true;
                    }
                }
                if in_fs
                    && path.len() >= 2
                    && path[path.len() - 1] == "binary"
                    && path[path.len() - 2] == "filesystem"
                    && n == "lock"
                {
                    if get_attr(&a, "posixlock").as_deref() == Some("on") || get_attr(&a, "posix").as_deref() == Some("on") {
                        posix_lock = true;
                    }
                    if get_attr(&a, "flock").as_deref() == Some("on") {
                        flock = true;
                    }
                }
            }
            Ok(Event::End(e)) => {
                let n = utf8_name_end(&e);
                if n == "filesystem" && in_fs {
                    if let Some(dt) = driver_type.take() {
                        out.push(FilesystemConfig {
                            driver_type: dt,
                            source_dir: std::mem::take(&mut source_dir),
                            target_dir: std::mem::take(&mut target_dir),
                            accessmode: accessmode.take(),
                            readonly,
                            multidevs: multidevs.take(),
                            queue_size: queue_size.take(),
                            xattr,
                            posix_lock,
                            flock,
                            binary_path: binary_path.take(),
                        });
                    }
                    in_fs = false;
                    readonly = false;
                    xattr = false;
                    posix_lock = false;
                    flock = false;
                    source_dir.clear();
                    target_dir.clear();
                    accessmode = None;
                    multidevs = None;
                    driver_type = None;
                    queue_size = None;
                    binary_path = None;
                }
                path.pop();
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

pub fn parse_shmems(xml: &str) -> Result<Vec<ShmemConfig>, VirtManagerError> {
    let mut r = mk_reader(xml);
    let mut buf = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut out: Vec<ShmemConfig> = Vec::new();

    let mut in_shmem = false;
    let mut name = String::new();
    let mut model = ShmemModel::IvshmemPlain;
    let mut role = ShmemRole::Peer;
    let mut server: Option<String> = None;
    let mut size_unit_pending: Option<String> = None;
    let mut size_bytes: u64 = 0;
    let mut capturing_size = false;

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(xml_err(e, r.buffer_position())),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if n == "shmem" {
                    in_shmem = true;
                    name = get_attr(&a, "name").unwrap_or_default();
                    role = get_attr(&a, "role").and_then(|s| ShmemRole::parse(&s)).unwrap_or(ShmemRole::Peer);
                }
                if in_shmem && path.last().map(String::as_str) == Some("shmem") {
                    match n.as_str() {
                        "model" => {
                            if let Some(t) = get_attr(&a, "type") {
                                if let Some(m) = ShmemModel::parse(&t) { model = m; }
                            }
                        }
                        "server" => {
                            if let Some(p) = get_attr(&a, "path") { server = Some(p); }
                        }
                        "size" => {
                            size_unit_pending = Some(get_attr(&a, "unit").unwrap_or_else(|| "M".into()));
                            capturing_size = true;
                        }
                        _ => {}
                    }
                }
                path.push(n);
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if in_shmem && path.last().map(String::as_str) == Some("shmem") {
                    match n.as_str() {
                        "model" => {
                            if let Some(t) = get_attr(&a, "type") {
                                if let Some(m) = ShmemModel::parse(&t) { model = m; }
                            }
                        }
                        "server" => {
                            if let Some(p) = get_attr(&a, "path") { server = Some(p); }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                let n = utf8_name_end(&e);
                if n == "size" && capturing_size { capturing_size = false; }
                if n == "shmem" && in_shmem {
                    out.push(ShmemConfig {
                        name: std::mem::take(&mut name),
                        size_bytes,
                        model,
                        role,
                        server: server.take(),
                    });
                    in_shmem = false;
                    size_bytes = 0;
                    model = ShmemModel::IvshmemPlain;
                    role = ShmemRole::Peer;
                    size_unit_pending = None;
                }
                path.pop();
            }
            Ok(Event::Text(t)) => {
                if capturing_size {
                    let txt = t.unescape().unwrap_or_default().trim().to_string();
                    if let Ok(val) = txt.parse::<u64>() {
                        let unit = size_unit_pending.clone().unwrap_or_else(|| "M".into());
                        size_bytes = scale_to_bytes(val, &unit);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

fn scale_to_bytes(val: u64, unit: &str) -> u64 {
    match unit {
        "B" | "bytes" => val,
        "KB" => val * 1000,
        "K" | "KiB" => val * 1024,
        "MB" => val * 1000 * 1000,
        "M" | "MiB" => val * 1024 * 1024,
        "GB" => val * 1000 * 1000 * 1000,
        "G" | "GiB" => val * 1024 * 1024 * 1024,
        "TB" => val * 1000u64.pow(4),
        "T" | "TiB" => val * 1024u64.pow(4),
        _ => val * 1024 * 1024,
    }
}

pub fn build_filesystem_xml(fs: &FilesystemConfig) -> Result<String, VirtManagerError> {
    validate_filesystem(fs)?;
    let mut s = String::new();
    s.push_str("<filesystem type='mount'");
    if fs.driver_type != FilesystemDriver::Virtiofs {
        if let Some(a) = fs.accessmode {
            s.push_str(&format!(" accessmode='{}'", a.as_str()));
        }
    }
    if let Some(m) = fs.multidevs {
        s.push_str(&format!(" multidevs='{}'", m.as_str()));
    }
    s.push_str(">\n");

    if fs.driver_type == FilesystemDriver::Virtiofs {
        if let Some(q) = fs.queue_size {
            s.push_str(&format!("  <driver type='virtiofs' queue='{}'/>\n", q));
        } else {
            s.push_str("  <driver type='virtiofs'/>\n");
        }
        let needs_binary = fs.binary_path.is_some() || fs.xattr || fs.posix_lock || fs.flock;
        if needs_binary {
            s.push_str("  <binary");
            if let Some(bp) = &fs.binary_path {
                s.push_str(&format!(" path='{}'", escape_xml(bp)));
            }
            if fs.xattr {
                s.push_str(" xattr='on'");
            }
            if fs.posix_lock || fs.flock {
                s.push_str(">\n");
                s.push_str("    <lock");
                if fs.posix_lock {
                    s.push_str(" posixlock='on'");
                }
                if fs.flock {
                    s.push_str(" flock='on'");
                }
                s.push_str("/>\n");
                s.push_str("  </binary>\n");
            } else {
                s.push_str("/>\n");
            }
        }
    } else {
        s.push_str(&format!("  <driver type='{}'/>\n", fs.driver_type.as_str()));
    }

    s.push_str(&format!("  <source dir='{}'/>\n", escape_xml(&fs.source_dir)));
    s.push_str(&format!("  <target dir='{}'/>\n", escape_xml(&fs.target_dir)));
    if fs.readonly {
        s.push_str("  <readonly/>\n");
    }
    s.push_str("</filesystem>\n");
    Ok(s)
}

pub fn build_shmem_xml(sh: &ShmemConfig) -> Result<String, VirtManagerError> {
    if sh.name.is_empty() {
        return Err(invalid("shmem name must not be empty"));
    }
    if sh.size_bytes == 0 {
        return Err(invalid("shmem size_bytes must be > 0"));
    }
    let (val, unit) = if sh.size_bytes % (1024 * 1024) == 0 {
        (sh.size_bytes / (1024 * 1024), "M")
    } else if sh.size_bytes % 1024 == 0 {
        (sh.size_bytes / 1024, "KiB")
    } else {
        (sh.size_bytes, "B")
    };

    let mut s = String::new();
    s.push_str(&format!("<shmem name='{}' role='{}'>\n", escape_xml(&sh.name), sh.role.as_str()));
    s.push_str(&format!("  <model type='{}'/>\n", sh.model.as_str()));
    s.push_str(&format!("  <size unit='{}'>{}</size>\n", unit, val));
    if let Some(path) = &sh.server {
        s.push_str(&format!("  <server path='{}'/>\n", escape_xml(path)));
    }
    s.push_str("</shmem>\n");
    Ok(s)
}

pub fn apply_add_filesystem(xml: &str, fs: &FilesystemConfig) -> Result<String, VirtManagerError> {
    let fragment = build_filesystem_xml(fs)?;
    insert_into_devices(xml, &fragment)
}

pub fn apply_remove_filesystem(xml: &str, target_dir: &str) -> Result<String, VirtManagerError> {
    remove_element_matching(xml, "filesystem", |body| {
        body_target_dir(body).as_deref() == Some(target_dir)
    })
}

pub fn apply_update_filesystem(xml: &str, fs: &FilesystemConfig) -> Result<String, VirtManagerError> {
    let without = apply_remove_filesystem(xml, &fs.target_dir)?;
    apply_add_filesystem(&without, fs)
}

pub fn apply_add_shmem(xml: &str, sh: &ShmemConfig) -> Result<String, VirtManagerError> {
    let fragment = build_shmem_xml(sh)?;
    insert_into_devices(xml, &fragment)
}

pub fn apply_remove_shmem(xml: &str, name: &str) -> Result<String, VirtManagerError> {
    remove_element_matching(xml, "shmem", |head| {
        attr_value_in_start(head, "name").as_deref() == Some(name)
    })
}

pub fn has_shared_memory_backing(xml: &str) -> bool {
    let mut r = mk_reader(xml);
    let mut buf = Vec::new();
    let mut in_mb = false;
    loop {
        match r.read_event_into(&mut buf) {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                if utf8_name(&e) == "memoryBacking" { in_mb = true; }
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if in_mb && n == "access" && get_attr(&a, "mode").as_deref() == Some("shared") {
                    return true;
                }
            }
            Ok(Event::End(e)) => {
                if utf8_name_end(&e) == "memoryBacking" { in_mb = false; }
            }
            _ => {}
        }
        buf.clear();
    }
    false
}

pub fn apply_enable_shared_memory_backing(xml: &str) -> Result<String, VirtManagerError> {
    if has_shared_memory_backing(xml) {
        return Ok(xml.to_string());
    }
    let injected = "<memoryBacking>\n    <access mode='shared'/>\n  </memoryBacking>";
    if let Some((s, e)) = find_element(xml, "memoryBacking") {
        let mut out = String::with_capacity(xml.len() + injected.len());
        out.push_str(&xml[..s]);
        out.push_str(injected);
        out.push_str(&xml[e..]);
        Ok(out)
    } else if let Some(idx) = xml.rfind("</domain>") {
        let mut out = String::with_capacity(xml.len() + injected.len() + 4);
        out.push_str(&xml[..idx]);
        out.push_str("  ");
        out.push_str(injected);
        out.push('\n');
        out.push_str(&xml[idx..]);
        Ok(out)
    } else {
        Err(VirtManagerError::XmlParsingFailed { reason: "no </domain> found".into() })
    }
}

pub fn apply_remove_memory_backing(xml: &str) -> Result<String, VirtManagerError> {
    if let Some((s, e)) = find_element(xml, "memoryBacking") {
        let mut end = e;
        let bytes = xml.as_bytes();
        while end < bytes.len() && (bytes[end] == b'\n' || bytes[end] == b' ') {
            end += 1;
        }
        let mut start = s;
        while start > 0 && (xml.as_bytes()[start - 1] == b' ' || xml.as_bytes()[start - 1] == b'\t') {
            start -= 1;
        }
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..start]);
        out.push_str(&xml[end..]);
        Ok(out)
    } else {
        Ok(xml.to_string())
    }
}

fn insert_into_devices(xml: &str, fragment: &str) -> Result<String, VirtManagerError> {
    if let Some(idx) = xml.rfind("</devices>") {
        let indented = indent_fragment(fragment, "    ");
        let mut out = String::with_capacity(xml.len() + indented.len());
        out.push_str(&xml[..idx]);
        out.push_str(&indented);
        out.push_str(&xml[idx..]);
        return Ok(out);
    }
    if let Some(idx) = xml.rfind("</domain>") {
        let indented = indent_fragment(fragment, "    ");
        let mut out = String::with_capacity(xml.len() + indented.len() + 32);
        out.push_str(&xml[..idx]);
        out.push_str("  <devices>\n");
        out.push_str(&indented);
        out.push_str("  </devices>\n");
        out.push_str(&xml[idx..]);
        Ok(out)
    } else {
        Err(VirtManagerError::XmlParsingFailed { reason: "neither </devices> nor </domain> found".into() })
    }
}

fn indent_fragment(fragment: &str, indent: &str) -> String {
    let mut out = String::with_capacity(fragment.len() + fragment.lines().count() * indent.len());
    for line in fragment.lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            out.push_str(indent);
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn remove_element_matching<F>(xml: &str, name: &str, pred: F) -> Result<String, VirtManagerError>
where
    F: Fn(&str) -> bool,
{
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth: i32 = 0;
    let mut start_byte: Option<usize> = None;
    let mut end_byte: Option<usize> = None;
    let mut scanning_inside = false;

    loop {
        let pos_before = r.buffer_position() as usize;
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(VirtManagerError::XmlParsingFailed { reason: e.to_string() }),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                if n == name && !scanning_inside {
                    start_byte = Some(pos_before);
                    depth = 1;
                    scanning_inside = true;
                } else if scanning_inside && n == name {
                    depth += 1;
                }
            }
            Ok(Event::End(e)) => {
                if scanning_inside && utf8_name_end(&e) == name {
                    depth -= 1;
                    if depth == 0 {
                        let end = r.buffer_position() as usize;
                        if let Some(s) = start_byte {
                            if pred(&xml[s..end]) {
                                end_byte = Some(end);
                                break;
                            }
                        }
                        start_byte = None;
                        scanning_inside = false;
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    if let (Some(s), Some(e)) = (start_byte, end_byte) {
        let mut end = e;
        let bytes = xml.as_bytes();
        while end < bytes.len() && bytes[end] == b'\n' { end += 1; }
        let mut start = s;
        while start > 0 && (xml.as_bytes()[start - 1] == b' ' || xml.as_bytes()[start - 1] == b'\t') {
            start -= 1;
        }
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..start]);
        out.push_str(&xml[end..]);
        Ok(out)
    } else {
        Ok(xml.to_string())
    }
}

fn find_element(xml: &str, name: &str) -> Option<(usize, usize)> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth: i32 = 0;
    let mut start_byte: Option<usize> = None;
    loop {
        let pos_before = r.buffer_position() as usize;
        match r.read_event_into(&mut buf) {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                if utf8_name(&e) == name {
                    if depth == 0 { start_byte = Some(pos_before); }
                    depth += 1;
                }
            }
            Ok(Event::Empty(e)) => {
                if utf8_name(&e) == name && depth == 0 {
                    return Some((pos_before, r.buffer_position() as usize));
                }
            }
            Ok(Event::End(e)) => {
                if utf8_name_end(&e) == name {
                    depth -= 1;
                    if depth == 0 {
                        return start_byte.map(|s| (s, r.buffer_position() as usize));
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    None
}

fn body_target_dir(body: &str) -> Option<String> {
    use regex::Regex;
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"<target\s+[^>]*?dir=['"]([^'"]+)['"]"#).unwrap()
    });
    RE.captures(body).and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn attr_value_in_start(head: &str, attr: &str) -> Option<String> {
    use regex::Regex;
    let pat = format!(r#"{}=['"]([^'"]+)['"]"#, regex::escape(attr));
    let re = Regex::new(&pat).ok()?;
    re.captures(head).and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn mk_reader(xml: &str) -> Reader<&[u8]> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    r
}

fn xml_err(e: quick_xml::Error, pos: u64) -> VirtManagerError {
    VirtManagerError::XmlParsingFailed { reason: format!("at {pos}: {e}") }
}

fn utf8_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn utf8_name_end(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes().filter_map(|a| a.ok()).map(|a| (
        String::from_utf8_lossy(a.key.as_ref()).to_string(),
        a.unescape_value().unwrap_or_default().to_string(),
    )).collect()
}

fn get_attr(attrs: &[(String, String)], key: &str) -> Option<String> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN_WITH_VIRTIOFS: &str = r#"<domain type='kvm'>
  <name>t</name>
  <memoryBacking>
    <access mode='shared'/>
  </memoryBacking>
  <devices>
    <filesystem type='mount'>
      <driver type='virtiofs' queue='1024'/>
      <binary path='/usr/libexec/virtiofsd' xattr='on'>
        <lock posixlock='on' flock='on'/>
      </binary>
      <source dir='/tmp/hostshare'/>
      <target dir='shared'/>
      <readonly/>
    </filesystem>
  </devices>
</domain>
"#;

    const DOMAIN_WITH_9P: &str = r#"<domain>
  <devices>
    <filesystem type='mount' accessmode='mapped' multidevs='remap'>
      <driver type='path'/>
      <source dir='/srv/data'/>
      <target dir='data'/>
    </filesystem>
  </devices>
</domain>
"#;

    const DOMAIN_WITH_SHMEM: &str = r#"<domain>
  <devices>
    <shmem name='ivshmem-peer' role='peer'>
      <model type='ivshmem-plain'/>
      <size unit='M'>64</size>
    </shmem>
    <shmem name='ivshmem-db' role='master'>
      <model type='ivshmem-doorbell'/>
      <size unit='M'>128</size>
      <server path='/var/lib/libvirt/shmem/db.sock'/>
    </shmem>
  </devices>
</domain>
"#;

    const DOMAIN_EMPTY: &str = r#"<domain>
  <devices>
  </devices>
</domain>
"#;

    #[test]
    fn parses_virtiofs_filesystem() {
        let fs = parse_filesystems(DOMAIN_WITH_VIRTIOFS).unwrap();
        assert_eq!(fs.len(), 1);
        let f = &fs[0];
        assert_eq!(f.driver_type, FilesystemDriver::Virtiofs);
        assert_eq!(f.source_dir, "/tmp/hostshare");
        assert_eq!(f.target_dir, "shared");
        assert_eq!(f.queue_size, Some(1024));
        assert!(f.xattr);
        assert!(f.posix_lock);
        assert!(f.flock);
        assert_eq!(f.binary_path.as_deref(), Some("/usr/libexec/virtiofsd"));
        assert!(f.readonly);
        assert!(f.accessmode.is_none());
    }

    #[test]
    fn parses_9p_filesystem() {
        let fs = parse_filesystems(DOMAIN_WITH_9P).unwrap();
        assert_eq!(fs.len(), 1);
        let f = &fs[0];
        assert_eq!(f.driver_type, FilesystemDriver::Path);
        assert_eq!(f.accessmode, Some(FilesystemAccessMode::Mapped));
        assert_eq!(f.multidevs, Some(MultidevsMode::Remap));
        assert_eq!(f.source_dir, "/srv/data");
        assert_eq!(f.target_dir, "data");
        assert!(!f.readonly);
    }

    #[test]
    fn round_trip_virtiofs() {
        let fs = FilesystemConfig {
            driver_type: FilesystemDriver::Virtiofs,
            source_dir: "/mnt/host".into(),
            target_dir: "rt".into(),
            accessmode: None, readonly: false, multidevs: None,
            queue_size: Some(512), xattr: true, posix_lock: true, flock: false,
            binary_path: None,
        };
        let xml = build_filesystem_xml(&fs).unwrap();
        let wrap = format!("<domain><devices>{}</devices></domain>", xml);
        let back = parse_filesystems(&wrap).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], fs);
    }

    #[test]
    fn round_trip_9p() {
        let fs = FilesystemConfig {
            driver_type: FilesystemDriver::Path,
            source_dir: "/srv".into(), target_dir: "srv".into(),
            accessmode: Some(FilesystemAccessMode::Passthrough),
            readonly: true, multidevs: Some(MultidevsMode::Forbid),
            queue_size: None, xattr: false, posix_lock: false, flock: false,
            binary_path: None,
        };
        let xml = build_filesystem_xml(&fs).unwrap();
        let wrap = format!("<domain><devices>{}</devices></domain>", xml);
        let back = parse_filesystems(&wrap).unwrap();
        assert_eq!(back[0], fs);
    }

    #[test]
    fn accessmode_rejected_on_virtiofs() {
        let mut fs = FilesystemConfig::virtiofs("/a", "b");
        fs.accessmode = Some(FilesystemAccessMode::Mapped);
        let err = build_filesystem_xml(&fs).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("accessmode"));
    }

    #[test]
    fn injection_safe_escape() {
        let fs = FilesystemConfig {
            driver_type: FilesystemDriver::Virtiofs,
            source_dir: "/tmp/a'&<b>\"evil".into(),
            target_dir: "tag'\"&<>".into(),
            accessmode: None, readonly: false, multidevs: None,
            queue_size: None, xattr: false, posix_lock: false, flock: false,
            binary_path: Some("/bin/ls;rm -rf /\"evil".into()),
        };
        let xml = build_filesystem_xml(&fs).unwrap();
        assert!(!xml.contains("evil\""));
        assert!(xml.contains("&apos;"));
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&lt;b&gt;"));
        assert!(xml.contains("&quot;"));
        let wrap = format!("<domain><devices>{}</devices></domain>", xml);
        let back = parse_filesystems(&wrap).unwrap();
        assert_eq!(back[0].source_dir, "/tmp/a'&<b>\"evil");
        assert_eq!(back[0].target_dir, "tag'\"&<>");
    }

    #[test]
    fn multiple_filesystems_preserve_order() {
        let xml = r#"<domain><devices>
          <filesystem type='mount'><driver type='virtiofs'/><source dir='/a'/><target dir='first'/></filesystem>
          <filesystem type='mount'><driver type='virtiofs'/><source dir='/b'/><target dir='second'/></filesystem>
          <filesystem type='mount' accessmode='passthrough'><driver type='path'/><source dir='/c'/><target dir='third'/></filesystem>
        </devices></domain>"#;
        let fs = parse_filesystems(xml).unwrap();
        assert_eq!(fs.len(), 3);
        assert_eq!(fs[0].target_dir, "first");
        assert_eq!(fs[1].target_dir, "second");
        assert_eq!(fs[2].target_dir, "third");
        assert_eq!(fs[2].driver_type, FilesystemDriver::Path);
    }

    #[test]
    fn shmem_parse_and_build() {
        let shs = parse_shmems(DOMAIN_WITH_SHMEM).unwrap();
        assert_eq!(shs.len(), 2);
        assert_eq!(shs[0].name, "ivshmem-peer");
        assert_eq!(shs[0].role, ShmemRole::Peer);
        assert_eq!(shs[0].model, ShmemModel::IvshmemPlain);
        assert_eq!(shs[0].size_bytes, 64 * 1024 * 1024);
        assert!(shs[0].server.is_none());
        assert_eq!(shs[1].name, "ivshmem-db");
        assert_eq!(shs[1].role, ShmemRole::Master);
        assert_eq!(shs[1].model, ShmemModel::IvshmemDoorbell);
        assert_eq!(shs[1].size_bytes, 128 * 1024 * 1024);
        assert_eq!(shs[1].server.as_deref(), Some("/var/lib/libvirt/shmem/db.sock"));

        let xml = build_shmem_xml(&shs[1]).unwrap();
        let wrap = format!("<domain><devices>{}</devices></domain>", xml);
        let back = parse_shmems(&wrap).unwrap();
        assert_eq!(back[0], shs[1]);
    }

    #[test]
    fn shmem_size_unit_scaling() {
        let xml = r#"<domain><devices>
            <shmem name='a' role='peer'><model type='ivshmem-plain'/><size unit='K'>1024</size></shmem>
            <shmem name='b' role='peer'><model type='ivshmem-plain'/><size unit='G'>1</size></shmem>
            <shmem name='c' role='peer'><model type='ivshmem-plain'/><size unit='B'>4096</size></shmem>
        </devices></domain>"#;
        let shs = parse_shmems(xml).unwrap();
        assert_eq!(shs[0].size_bytes, 1024 * 1024);
        assert_eq!(shs[1].size_bytes, 1024 * 1024 * 1024);
        assert_eq!(shs[2].size_bytes, 4096);
    }

    #[test]
    fn empty_input_returns_empty_vecs() {
        assert!(parse_filesystems(DOMAIN_EMPTY).unwrap().is_empty());
        assert!(parse_shmems(DOMAIN_EMPTY).unwrap().is_empty());
        assert!(parse_filesystems("").unwrap().is_empty());
        assert!(parse_shmems("").unwrap().is_empty());
    }

    #[test]
    fn apply_add_then_remove() {
        let xml0 = DOMAIN_EMPTY.to_string();
        let fs = FilesystemConfig::virtiofs("/tmp/share", "shared");
        let xml1 = apply_add_filesystem(&xml0, &fs).unwrap();
        let parsed = parse_filesystems(&xml1).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].target_dir, "shared");

        let xml2 = apply_remove_filesystem(&xml1, "shared").unwrap();
        let parsed = parse_filesystems(&xml2).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn apply_update_swaps_body() {
        let xml0 = DOMAIN_EMPTY.to_string();
        let fs1 = FilesystemConfig {
            source_dir: "/src/one".into(),
            ..FilesystemConfig::virtiofs("/tmp", "tag")
        };
        let xml1 = apply_add_filesystem(&xml0, &fs1).unwrap();
        let fs2 = FilesystemConfig {
            source_dir: "/src/two".into(),
            readonly: true,
            ..fs1.clone()
        };
        let xml2 = apply_update_filesystem(&xml1, &fs2).unwrap();
        let parsed = parse_filesystems(&xml2).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].source_dir, "/src/two");
        assert!(parsed[0].readonly);
    }

    #[test]
    fn detects_shared_memory_backing() {
        assert!(has_shared_memory_backing(DOMAIN_WITH_VIRTIOFS));
        assert!(!has_shared_memory_backing(DOMAIN_EMPTY));
        let other = r#"<domain><memoryBacking><access mode='private'/></memoryBacking></domain>"#;
        assert!(!has_shared_memory_backing(other));
    }

    #[test]
    fn enable_shared_memory_backing_idempotent() {
        let xml1 = apply_enable_shared_memory_backing(DOMAIN_EMPTY).unwrap();
        assert!(has_shared_memory_backing(&xml1));
        let xml2 = apply_enable_shared_memory_backing(&xml1).unwrap();
        let count = xml2.matches("<memoryBacking").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn shmem_apply_add_remove() {
        let xml0 = DOMAIN_EMPTY.to_string();
        let sh = ShmemConfig {
            name: "test".into(), size_bytes: 32 * 1024 * 1024,
            model: ShmemModel::IvshmemPlain, role: ShmemRole::Peer, server: None,
        };
        let xml1 = apply_add_shmem(&xml0, &sh).unwrap();
        let list = parse_shmems(&xml1).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "test");
        assert_eq!(list[0].size_bytes, 32 * 1024 * 1024);

        let xml2 = apply_remove_shmem(&xml1, "test").unwrap();
        let list = parse_shmems(&xml2).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn validate_rejects_empty_paths() {
        let mut fs = FilesystemConfig::virtiofs("", "tag");
        assert!(validate_filesystem(&fs).is_err());
        fs.source_dir = "/tmp".into();
        fs.target_dir = "".into();
        assert!(validate_filesystem(&fs).is_err());
    }

    #[test]
    fn virtiofs_flags_rejected_on_9p() {
        let fs = FilesystemConfig {
            driver_type: FilesystemDriver::Path,
            source_dir: "/a".into(), target_dir: "b".into(),
            accessmode: Some(FilesystemAccessMode::Passthrough),
            readonly: false, multidevs: None,
            queue_size: Some(256), xattr: false, posix_lock: false, flock: false,
            binary_path: None,
        };
        let err = build_filesystem_xml(&fs).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("virtiofs"));
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        let out = apply_remove_filesystem(DOMAIN_EMPTY, "no-such").unwrap();
        assert_eq!(out, DOMAIN_EMPTY);
        let out = apply_remove_shmem(DOMAIN_EMPTY, "no-such").unwrap();
        assert_eq!(out, DOMAIN_EMPTY);
    }

    #[test]
    fn shmem_build_odd_size_uses_bytes() {
        let sh = ShmemConfig {
            name: "odd".into(), size_bytes: 4097,
            model: ShmemModel::IvshmemPlain, role: ShmemRole::Peer, server: None,
        };
        let xml = build_shmem_xml(&sh).unwrap();
        assert!(xml.contains("unit='B'"));
        assert!(xml.contains(">4097<"));
    }

    #[test]
    fn shmem_build_includes_server_path() {
        let sh = ShmemConfig {
            name: "db".into(), size_bytes: 64 * 1024 * 1024,
            model: ShmemModel::IvshmemDoorbell, role: ShmemRole::Master,
            server: Some("/var/run/ivshmem.sock".into()),
        };
        let xml = build_shmem_xml(&sh).unwrap();
        assert!(xml.contains("<server path='/var/run/ivshmem.sock'/>"));
    }
}
