//! Serial / console / parallel / channel character device editor.
//!
//! Libvirt groups `<serial>`, `<console>`, `<parallel>`, and
//! `<channel>` under a common "chardev" backing: all four share the
//! same source types (pty, unix, tcp, spicevmc, etc.) but differ in
//! their `<target>` semantics. We model the shared source as
//! `CharDeviceType` and give each category its own struct for the
//! target-specific fields.
//!
//! The high-value items here are the qemu-guest-agent and SPICE
//! vdagent channels. Both are well-known presets and have one-call
//! helpers (`guest_agent_channel()` / `spice_vdagent_channel()`).
//!
//! Apply strategy follows Round A (boot_config.rs): mutate the
//! existing XML in place rather than reserialising the whole domain,
//! so untouched sections round-trip exactly.

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

// ────────── types ──────────

/// The backing for a character device. Mirrors `<serial type='...'>`
/// and friends. Variants we don't understand are skipped at parse
/// time rather than erroring — preserves forward compatibility with
/// libvirt adding new source types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CharDeviceType {
    /// `<serial type='pty'/>` — libvirt allocates `/dev/pts/N`.
    Pty,
    /// `<serial type='dev'><source path='/dev/...'/>` — host tty.
    Dev { path: String },
    /// `<serial type='file'><source path='...' append='on|off'/>`.
    File { path: String, append: bool },
    /// `<serial type='pipe'><source path='...'/>` — named pipe.
    Pipe { path: String },
    /// `<serial type='tcp'><source mode='connect|bind' host='...'
    ///   service='...'/><protocol type='raw|telnet|tls'/>`.
    Tcp {
        host: String,
        port: u16,
        mode: TcpMode,
        protocol: TcpProtocol,
    },
    /// `<serial type='udp'><source mode='connect|bind' host host2/></serial>`.
    Udp {
        host: String,
        port: u16,
        bind_host: Option<String>,
        bind_port: Option<u16>,
    },
    /// `<serial type='unix'><source mode='connect|bind' path='...'/>`.
    Unix { path: String, mode: UnixMode },
    /// FreeBSD null-modem pair. Rare but documented in libvirt.
    Nmdm { master: String, slave: String },
    /// `<channel type='spicevmc'/>` — SPICE virtual machine channel.
    Spicevmc,
    /// `<channel type='spiceport'><source channel='...'/></channel>`.
    Spiceport { channel: String },
    /// `<channel type='dbus'><source channel='...'/></channel>`.
    Dbus { channel: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TcpMode { Connect, Bind }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TcpProtocol { Raw, Telnet, Tls }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UnixMode { Connect, Bind }

/// `<serial>` device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SerialConfig {
    pub source: CharDeviceType,
    /// isa-serial / usb-serial / pci-serial / sclp-serial
    pub target_type: String,
    pub target_port: Option<u32>,
}

/// `<console>` device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsoleConfig {
    pub source: CharDeviceType,
    /// serial / virtio / xen / sclp / sclplm
    pub target_type: String,
    pub target_port: Option<u32>,
}

/// `<parallel>` device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParallelConfig {
    pub source: CharDeviceType,
    pub target_port: Option<u32>,
}

/// `<channel>` device. `target_name` is the guest-visible port name
/// (e.g. `org.qemu.guest_agent.0`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelConfig {
    pub source: CharDeviceType,
    /// virtio / guestfwd / xen
    pub target_type: String,
    pub target_name: Option<String>,
}

/// Bundle returned from get_char_devices — one round-trip for the UI.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CharDevicesSnapshot {
    pub serials: Vec<SerialConfig>,
    pub consoles: Vec<ConsoleConfig>,
    pub channels: Vec<ChannelConfig>,
    pub parallels: Vec<ParallelConfig>,
}

// ────────── presets ──────────

/// Standard qemu-guest-agent channel. Libvirt auto-adds the
/// virtio-serial controller and a unix socket under
/// /run/libvirt/qemu/channel/.
pub fn guest_agent_channel() -> ChannelConfig {
    ChannelConfig {
        // Libvirt fills in the socket path automatically when type='unix'
        // and mode='bind' with no path — but kernels/virsh variants want
        // an explicit path occasionally. Emit a path of "" means libvirt
        // defaults it; we use an empty path to mean "let libvirt pick".
        source: CharDeviceType::Unix {
            path: String::new(),
            mode: UnixMode::Bind,
        },
        target_type: "virtio".into(),
        target_name: Some("org.qemu.guest_agent.0".into()),
    }
}

/// Standard SPICE vdagent channel.
pub fn spice_vdagent_channel() -> ChannelConfig {
    ChannelConfig {
        source: CharDeviceType::Spicevmc,
        target_type: "virtio".into(),
        target_name: Some("com.redhat.spice.0".into()),
    }
}

// ────────── parse ──────────

/// Raw `<serial|console|channel|parallel>` block with its parsed
/// source and target fields. Internal — the public API returns the
/// typed variants.
#[derive(Debug, Default)]
struct RawCharDev {
    el: String,                     // element name
    dev_type: String,               // type='pty'
    source_path: Option<String>,
    source_host: Option<String>,
    source_service: Option<String>,
    source_mode: Option<String>,
    source_append: Option<String>,
    source_channel: Option<String>,
    source_master: Option<String>,
    source_slave: Option<String>,
    // UDP has two <source> elements with different modes; keep the
    // second one separately.
    udp_bind_host: Option<String>,
    udp_bind_port: Option<String>,
    protocol_type: Option<String>,
    target_type: Option<String>,
    target_port: Option<String>,
    target_name: Option<String>,
}

/// Parse every `<serial>` block into a `SerialConfig`.
pub fn parse_serials(xml: &str) -> Result<Vec<SerialConfig>, VirtManagerError> {
    let raws = parse_all(xml, "serial")?;
    Ok(raws.into_iter().filter_map(to_serial).collect())
}

/// Parse every `<console>` block.
pub fn parse_consoles(xml: &str) -> Result<Vec<ConsoleConfig>, VirtManagerError> {
    let raws = parse_all(xml, "console")?;
    Ok(raws.into_iter().filter_map(to_console).collect())
}

/// Parse every `<channel>` block.
pub fn parse_channels(xml: &str) -> Result<Vec<ChannelConfig>, VirtManagerError> {
    let raws = parse_all(xml, "channel")?;
    Ok(raws.into_iter().filter_map(to_channel).collect())
}

/// Parse every `<parallel>` block.
pub fn parse_parallels(xml: &str) -> Result<Vec<ParallelConfig>, VirtManagerError> {
    let raws = parse_all(xml, "parallel")?;
    Ok(raws.into_iter().filter_map(to_parallel).collect())
}

fn parse_all(xml: &str, tag: &str) -> Result<Vec<RawCharDev>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut current: Option<RawCharDev> = None;
    let mut udp_source_count = 0u8;

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(VirtManagerError::XmlParsingFailed {
                reason: format!("at {}: {}", r.buffer_position(), e),
            }),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                handle_start_or_empty(&mut current, &mut udp_source_count, tag, &n, &a);
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                handle_start_or_empty(&mut current, &mut udp_source_count, tag, &n, &a);
            }
            Ok(Event::End(e)) => {
                let n = utf8_name_end(&e);
                if n == tag {
                    if let Some(raw) = current.take() {
                        out.push(raw);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn handle_start_or_empty(
    current: &mut Option<RawCharDev>,
    udp_source_count: &mut u8,
    tag: &str,
    n: &str,
    a: &[(String, String)],
) {
    if current.is_none() && n == tag {
        let mut raw = RawCharDev { el: tag.to_string(), ..Default::default() };
        if let Some(t) = get_attr(a, "type") {
            raw.dev_type = t;
        }
        *current = Some(raw);
        *udp_source_count = 0;
        return;
    }
    if let Some(raw) = current.as_mut() {
        apply_child(raw, n, a, udp_source_count);
    }
}

fn to_source(raw: &RawCharDev) -> Option<CharDeviceType> {
    match raw.dev_type.as_str() {
        "pty" => Some(CharDeviceType::Pty),
        "dev" => raw.source_path.clone().map(|p| CharDeviceType::Dev { path: p }),
        "file" => raw.source_path.clone().map(|p| CharDeviceType::File {
            path: p,
            append: raw.source_append.as_deref() == Some("on"),
        }),
        "pipe" => raw.source_path.clone().map(|p| CharDeviceType::Pipe { path: p }),
        "tcp" => {
            let host = raw.source_host.clone().unwrap_or_default();
            let port = raw.source_service.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
            let mode = match raw.source_mode.as_deref() {
                Some("bind") => TcpMode::Bind,
                _ => TcpMode::Connect,
            };
            let protocol = match raw.protocol_type.as_deref() {
                Some("telnet") => TcpProtocol::Telnet,
                Some("tls") => TcpProtocol::Tls,
                _ => TcpProtocol::Raw,
            };
            Some(CharDeviceType::Tcp { host, port, mode, protocol })
        }
        "udp" => {
            let host = raw.source_host.clone().unwrap_or_default();
            let port = raw.source_service.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
            let bind_host = raw.udp_bind_host.clone();
            let bind_port = raw.udp_bind_port.as_deref().and_then(|s| s.parse().ok());
            Some(CharDeviceType::Udp { host, port, bind_host, bind_port })
        }
        "unix" => raw.source_path.clone().map(|p| CharDeviceType::Unix {
            path: p,
            mode: match raw.source_mode.as_deref() {
                Some("connect") => UnixMode::Connect,
                _ => UnixMode::Bind,
            },
        }).or_else(|| {
            // unix with no path — qemu-ga style, libvirt fills it in
            Some(CharDeviceType::Unix {
                path: String::new(),
                mode: match raw.source_mode.as_deref() {
                    Some("connect") => UnixMode::Connect,
                    _ => UnixMode::Bind,
                },
            })
        }),
        "nmdm" => Some(CharDeviceType::Nmdm {
            master: raw.source_master.clone().unwrap_or_default(),
            slave: raw.source_slave.clone().unwrap_or_default(),
        }),
        "spicevmc" => Some(CharDeviceType::Spicevmc),
        "spiceport" => raw.source_channel.clone().map(|c| CharDeviceType::Spiceport { channel: c }),
        "dbus" => raw.source_channel.clone().map(|c| CharDeviceType::Dbus { channel: c }),
        _ => None, // unknown source type — skip gracefully
    }
}

fn to_serial(raw: RawCharDev) -> Option<SerialConfig> {
    let source = to_source(&raw)?;
    Some(SerialConfig {
        source,
        target_type: raw.target_type.unwrap_or_else(|| "isa-serial".into()),
        target_port: raw.target_port.and_then(|s| s.parse().ok()),
    })
}

fn to_console(raw: RawCharDev) -> Option<ConsoleConfig> {
    let source = to_source(&raw)?;
    Some(ConsoleConfig {
        source,
        target_type: raw.target_type.unwrap_or_else(|| "serial".into()),
        target_port: raw.target_port.and_then(|s| s.parse().ok()),
    })
}

fn to_channel(raw: RawCharDev) -> Option<ChannelConfig> {
    let source = to_source(&raw)?;
    Some(ChannelConfig {
        source,
        target_type: raw.target_type.unwrap_or_else(|| "virtio".into()),
        target_name: raw.target_name,
    })
}

fn to_parallel(raw: RawCharDev) -> Option<ParallelConfig> {
    let source = to_source(&raw)?;
    Some(ParallelConfig {
        source,
        target_port: raw.target_port.and_then(|s| s.parse().ok()),
    })
}

// ────────── build ──────────

/// Build the `<source>` (and `<protocol>`) children for a given source
/// type. Returns a fragment without any outer element, indented with
/// 2-space pad so the caller can concatenate inside a 4-space parent.
fn build_source_inner(src: &CharDeviceType) -> String {
    let mut s = String::new();
    match src {
        CharDeviceType::Pty => {}
        CharDeviceType::Dev { path } => {
            s.push_str(&format!("    <source path='{}'/>\n", escape_xml(path)));
        }
        CharDeviceType::File { path, append } => {
            s.push_str(&format!(
                "    <source path='{}' append='{}'/>\n",
                escape_xml(path),
                if *append { "on" } else { "off" },
            ));
        }
        CharDeviceType::Pipe { path } => {
            s.push_str(&format!("    <source path='{}'/>\n", escape_xml(path)));
        }
        CharDeviceType::Tcp { host, port, mode, protocol } => {
            let mode_str = match mode { TcpMode::Connect => "connect", TcpMode::Bind => "bind" };
            s.push_str(&format!(
                "    <source mode='{}' host='{}' service='{}'/>\n",
                mode_str, escape_xml(host), port,
            ));
            let proto = match protocol {
                TcpProtocol::Raw => "raw",
                TcpProtocol::Telnet => "telnet",
                TcpProtocol::Tls => "tls",
            };
            s.push_str(&format!("    <protocol type='{}'/>\n", proto));
        }
        CharDeviceType::Udp { host, port, bind_host, bind_port } => {
            s.push_str(&format!(
                "    <source mode='connect' host='{}' service='{}'/>\n",
                escape_xml(host), port,
            ));
            if bind_host.is_some() || bind_port.is_some() {
                s.push_str(&format!(
                    "    <source mode='bind' host='{}' service='{}'/>\n",
                    escape_xml(bind_host.as_deref().unwrap_or("")),
                    bind_port.unwrap_or(0),
                ));
            }
        }
        CharDeviceType::Unix { path, mode } => {
            let mode_str = match mode { UnixMode::Connect => "connect", UnixMode::Bind => "bind" };
            if path.is_empty() {
                // let libvirt pick the socket path
                s.push_str(&format!("    <source mode='{}'/>\n", mode_str));
            } else {
                s.push_str(&format!(
                    "    <source mode='{}' path='{}'/>\n",
                    mode_str, escape_xml(path),
                ));
            }
        }
        CharDeviceType::Nmdm { master, slave } => {
            s.push_str(&format!(
                "    <source master='{}' slave='{}'/>\n",
                escape_xml(master), escape_xml(slave),
            ));
        }
        CharDeviceType::Spicevmc => {}
        CharDeviceType::Spiceport { channel } => {
            s.push_str(&format!("    <source channel='{}'/>\n", escape_xml(channel)));
        }
        CharDeviceType::Dbus { channel } => {
            s.push_str(&format!("    <source channel='{}'/>\n", escape_xml(channel)));
        }
    }
    s
}

fn dev_type_str(src: &CharDeviceType) -> &'static str {
    match src {
        CharDeviceType::Pty => "pty",
        CharDeviceType::Dev { .. } => "dev",
        CharDeviceType::File { .. } => "file",
        CharDeviceType::Pipe { .. } => "pipe",
        CharDeviceType::Tcp { .. } => "tcp",
        CharDeviceType::Udp { .. } => "udp",
        CharDeviceType::Unix { .. } => "unix",
        CharDeviceType::Nmdm { .. } => "nmdm",
        CharDeviceType::Spicevmc => "spicevmc",
        CharDeviceType::Spiceport { .. } => "spiceport",
        CharDeviceType::Dbus { .. } => "dbus",
    }
}

/// Build a complete `<serial>` element.
pub fn build_serial(cfg: &SerialConfig) -> String {
    let mut s = format!("<serial type='{}'>\n", dev_type_str(&cfg.source));
    s.push_str(&build_source_inner(&cfg.source));
    let port_attr = match cfg.target_port {
        Some(p) => format!(" port='{}'", p),
        None => String::new(),
    };
    s.push_str(&format!(
        "    <target type='{}'{}/>\n",
        escape_xml(&cfg.target_type),
        port_attr,
    ));
    s.push_str("  </serial>");
    s
}

pub fn build_console(cfg: &ConsoleConfig) -> String {
    let mut s = format!("<console type='{}'>\n", dev_type_str(&cfg.source));
    s.push_str(&build_source_inner(&cfg.source));
    let port_attr = match cfg.target_port {
        Some(p) => format!(" port='{}'", p),
        None => String::new(),
    };
    s.push_str(&format!(
        "    <target type='{}'{}/>\n",
        escape_xml(&cfg.target_type),
        port_attr,
    ));
    s.push_str("  </console>");
    s
}

pub fn build_parallel(cfg: &ParallelConfig) -> String {
    let mut s = format!("<parallel type='{}'>\n", dev_type_str(&cfg.source));
    s.push_str(&build_source_inner(&cfg.source));
    let port_attr = match cfg.target_port {
        Some(p) => format!(" port='{}'", p),
        None => String::new(),
    };
    s.push_str(&format!("    <target{}/>\n", port_attr));
    s.push_str("  </parallel>");
    s
}

pub fn build_channel(cfg: &ChannelConfig) -> String {
    let mut s = format!("<channel type='{}'>\n", dev_type_str(&cfg.source));
    s.push_str(&build_source_inner(&cfg.source));
    let name_attr = match &cfg.target_name {
        Some(n) => format!(" name='{}'", escape_xml(n)),
        None => String::new(),
    };
    s.push_str(&format!(
        "    <target type='{}'{}/>\n",
        escape_xml(&cfg.target_type),
        name_attr,
    ));
    s.push_str("  </channel>");
    s
}

// ────────── apply_add ──────────

/// Inject a single `<tag>` block immediately before `</devices>`.
fn inject_before_devices_close(xml: &str, block: &str) -> Result<String, VirtManagerError> {
    let idx = xml.rfind("</devices>").ok_or_else(|| VirtManagerError::XmlParsingFailed {
        reason: "no </devices> in domain XML".into(),
    })?;
    let mut out = String::with_capacity(xml.len() + block.len() + 8);
    out.push_str(&xml[..idx]);
    out.push_str("  ");
    out.push_str(block);
    out.push('\n');
    out.push_str(&xml[idx..]);
    Ok(out)
}

pub fn apply_add_serial(xml: &str, cfg: &SerialConfig) -> Result<String, VirtManagerError> {
    inject_before_devices_close(xml, &build_serial(cfg))
}

pub fn apply_add_console(xml: &str, cfg: &ConsoleConfig) -> Result<String, VirtManagerError> {
    inject_before_devices_close(xml, &build_console(cfg))
}

pub fn apply_add_channel(xml: &str, cfg: &ChannelConfig) -> Result<String, VirtManagerError> {
    inject_before_devices_close(xml, &build_channel(cfg))
}

pub fn apply_add_parallel(xml: &str, cfg: &ParallelConfig) -> Result<String, VirtManagerError> {
    inject_before_devices_close(xml, &build_parallel(cfg))
}

// ────────── apply_remove ──────────

/// Remove the first `<serial>` whose `<target port='N'>` matches `port`.
pub fn apply_remove_serial(xml: &str, port: u32) -> Result<String, VirtManagerError> {
    remove_by_match(xml, "serial", |raw| {
        raw.target_port.as_deref().and_then(|s| s.parse::<u32>().ok()) == Some(port)
    })
}

pub fn apply_remove_console(xml: &str, port: u32) -> Result<String, VirtManagerError> {
    remove_by_match(xml, "console", |raw| {
        raw.target_port.as_deref().and_then(|s| s.parse::<u32>().ok()) == Some(port)
    })
}

pub fn apply_remove_parallel(xml: &str, port: u32) -> Result<String, VirtManagerError> {
    remove_by_match(xml, "parallel", |raw| {
        raw.target_port.as_deref().and_then(|s| s.parse::<u32>().ok()) == Some(port)
    })
}

/// Remove the first `<channel>` whose `<target name='...'>` matches.
pub fn apply_remove_channel(xml: &str, target_name: &str) -> Result<String, VirtManagerError> {
    remove_by_match(xml, "channel", |raw| {
        raw.target_name.as_deref() == Some(target_name)
    })
}

/// Byte-range locator: walk the XML tracking <tag>...</tag> regions,
/// parse each one's attributes inline, and return the first range that
/// matches `pred`.
fn remove_by_match<F>(xml: &str, tag: &str, pred: F) -> Result<String, VirtManagerError>
where
    F: Fn(&RawCharDev) -> bool,
{
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth: i32 = 0;
    let mut current: Option<RawCharDev> = None;
    let mut start_byte: Option<usize> = None;
    let mut udp_source_count = 0u8;

    loop {
        let pos_before = r.buffer_position() as usize;
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(VirtManagerError::XmlParsingFailed { reason: e.to_string() }),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                if current.is_none() && n == tag {
                    let a = attrs(&e);
                    let mut raw = RawCharDev { el: tag.to_string(), ..Default::default() };
                    if let Some(t) = get_attr(&a, "type") { raw.dev_type = t; }
                    current = Some(raw);
                    start_byte = Some(pos_before);
                    depth = 1;
                    udp_source_count = 0;
                } else if current.is_some() {
                    depth += 1;
                    let a = attrs(&e);
                    apply_child(current.as_mut().unwrap(), &n, &a, &mut udp_source_count);
                }
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                if current.is_none() && n == tag {
                    // empty <tag/> — won't match anything useful, skip
                    continue;
                } else if let Some(raw) = current.as_mut() {
                    let a = attrs(&e);
                    apply_child(raw, &n, &a, &mut udp_source_count);
                }
            }
            Ok(Event::End(e)) => {
                let n = utf8_name_end(&e);
                if current.is_some() && n == tag {
                    let end_byte = r.buffer_position() as usize;
                    let raw = current.take().unwrap();
                    if pred(&raw) {
                        let mut out = String::with_capacity(xml.len());
                        let s = start_byte.unwrap();
                        // consume leading indentation whitespace on the same line
                        let mut trim_start = s;
                        while trim_start > 0 {
                            let b = xml.as_bytes()[trim_start - 1];
                            if b == b' ' || b == b'\t' { trim_start -= 1; } else { break; }
                        }
                        // also consume one trailing newline if present
                        let mut trim_end = end_byte;
                        if xml.as_bytes().get(trim_end) == Some(&b'\n') { trim_end += 1; }
                        out.push_str(&xml[..trim_start]);
                        out.push_str(&xml[trim_end..]);
                        return Ok(out);
                    }
                    start_byte = None;
                } else if current.is_some() {
                    depth -= 1;
                }
            }
            _ => {}
        }
        buf.clear();
    }
    // nothing matched — return unchanged
    Ok(xml.to_string())
}

fn apply_child(raw: &mut RawCharDev, n: &str, a: &[(String, String)], udp_source_count: &mut u8) {
    match n {
        "source" => {
            let is_udp = raw.dev_type == "udp";
            if is_udp && *udp_source_count >= 1 {
                raw.udp_bind_host = get_attr(a, "host");
                raw.udp_bind_port = get_attr(a, "service");
            } else {
                if let Some(p) = get_attr(a, "path") { raw.source_path = Some(p); }
                if let Some(h) = get_attr(a, "host") { raw.source_host = Some(h); }
                if let Some(s) = get_attr(a, "service") { raw.source_service = Some(s); }
                if let Some(m) = get_attr(a, "mode") { raw.source_mode = Some(m); }
                if let Some(a2) = get_attr(a, "append") { raw.source_append = Some(a2); }
                if let Some(c) = get_attr(a, "channel") { raw.source_channel = Some(c); }
                if let Some(m2) = get_attr(a, "master") { raw.source_master = Some(m2); }
                if let Some(s2) = get_attr(a, "slave") { raw.source_slave = Some(s2); }
            }
            if is_udp { *udp_source_count += 1; }
        }
        "protocol" => { raw.protocol_type = get_attr(a, "type"); }
        "target" => {
            raw.target_type = get_attr(a, "type");
            raw.target_port = get_attr(a, "port");
            raw.target_name = get_attr(a, "name");
        }
        _ => {}
    }
}

// ────────── helpers ──────────

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

fn get_attr(a: &[(String, String)], k: &str) -> Option<String> {
    a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
}

// ────────── tests ──────────

#[cfg(test)]
mod tests {
    use super::*;

    // Real fedora-workstation-style fixture.
    const FEDORA_XML: &str = r#"<domain type='kvm'>
  <name>fedora</name>
  <devices>
    <serial type='pty'>
      <source path='/dev/pts/1'/>
      <target type='isa-serial' port='0'>
        <model name='isa-serial'/>
      </target>
      <alias name='serial0'/>
    </serial>
    <console type='pty' tty='/dev/pts/1'>
      <source path='/dev/pts/1'/>
      <target type='serial' port='0'/>
      <alias name='serial0'/>
    </console>
    <channel type='unix'>
      <source mode='bind' path='/run/libvirt/qemu/channel/26-fedora-workstation/org.qemu.guest_agent.0'/>
      <target type='virtio' name='org.qemu.guest_agent.0' state='connected'/>
      <alias name='channel0'/>
      <address type='virtio-serial' controller='0' bus='0' port='1'/>
    </channel>
    <channel type='spicevmc'>
      <target type='virtio' name='com.redhat.spice.0' state='connected'/>
      <alias name='channel1'/>
      <address type='virtio-serial' controller='0' bus='0' port='2'/>
    </channel>
  </devices>
</domain>"#;

    #[test]
    fn parse_serial_pty_fedora() {
        let s = parse_serials(FEDORA_XML).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].source, CharDeviceType::Pty);
        assert_eq!(s[0].target_type, "isa-serial");
        assert_eq!(s[0].target_port, Some(0));
    }

    #[test]
    fn parse_console_pty_fedora() {
        let c = parse_consoles(FEDORA_XML).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].source, CharDeviceType::Pty);
        assert_eq!(c[0].target_type, "serial");
    }

    #[test]
    fn parse_channels_fedora() {
        let ch = parse_channels(FEDORA_XML).unwrap();
        assert_eq!(ch.len(), 2);
        // order preserved: qemu-ga first, then vdagent
        assert_eq!(ch[0].target_name.as_deref(), Some("org.qemu.guest_agent.0"));
        assert!(matches!(ch[0].source, CharDeviceType::Unix { .. }));
        assert_eq!(ch[1].target_name.as_deref(), Some("com.redhat.spice.0"));
        assert_eq!(ch[1].source, CharDeviceType::Spicevmc);
    }

    #[test]
    fn parse_each_source_type() {
        let xml = r#"<domain><devices>
            <serial type='pty'><target type='isa-serial' port='0'/></serial>
            <serial type='dev'><source path='/dev/ttyS1'/><target type='isa-serial' port='1'/></serial>
            <serial type='file'><source path='/tmp/log' append='on'/><target type='isa-serial' port='2'/></serial>
            <serial type='tcp'><source mode='bind' host='0.0.0.0' service='4001'/><protocol type='telnet'/><target type='isa-serial' port='3'/></serial>
            <serial type='udp'><source mode='connect' host='10.0.0.1' service='5000'/><source mode='bind' host='0.0.0.0' service='5001'/><target type='isa-serial' port='4'/></serial>
            <serial type='unix'><source mode='bind' path='/tmp/sock'/><target type='isa-serial' port='5'/></serial>
            <serial type='pipe'><source path='/tmp/pipe'/><target type='isa-serial' port='6'/></serial>
        </devices></domain>"#;
        let s = parse_serials(xml).unwrap();
        assert_eq!(s.len(), 7);
        assert_eq!(s[0].source, CharDeviceType::Pty);
        assert!(matches!(s[1].source, CharDeviceType::Dev { ref path } if path == "/dev/ttyS1"));
        assert!(matches!(s[2].source, CharDeviceType::File { ref path, append: true } if path == "/tmp/log"));
        assert!(matches!(s[3].source, CharDeviceType::Tcp { ref host, port: 4001, mode: TcpMode::Bind, protocol: TcpProtocol::Telnet } if host == "0.0.0.0"));
        assert!(matches!(s[4].source, CharDeviceType::Udp { ref host, port: 5000, .. } if host == "10.0.0.1"));
        if let CharDeviceType::Udp { ref bind_host, bind_port, .. } = s[4].source {
            assert_eq!(bind_host.as_deref(), Some("0.0.0.0"));
            assert_eq!(bind_port, Some(5001));
        }
        assert!(matches!(s[5].source, CharDeviceType::Unix { ref path, mode: UnixMode::Bind } if path == "/tmp/sock"));
        assert!(matches!(s[6].source, CharDeviceType::Pipe { ref path } if path == "/tmp/pipe"));
    }

    #[test]
    fn serial_round_trip() {
        let cfg = SerialConfig {
            source: CharDeviceType::Tcp {
                host: "127.0.0.1".into(),
                port: 9999,
                mode: TcpMode::Connect,
                protocol: TcpProtocol::Raw,
            },
            target_type: "isa-serial".into(),
            target_port: Some(0),
        };
        let wrapped = format!("<domain><devices>  {}\n</devices></domain>", build_serial(&cfg));
        let parsed = parse_serials(&wrapped).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], cfg);
    }

    #[test]
    fn console_round_trip() {
        let cfg = ConsoleConfig {
            source: CharDeviceType::Pty,
            target_type: "virtio".into(),
            target_port: Some(1),
        };
        let wrapped = format!("<domain><devices>  {}\n</devices></domain>", build_console(&cfg));
        let parsed = parse_consoles(&wrapped).unwrap();
        assert_eq!(parsed[0], cfg);
    }

    #[test]
    fn channel_round_trip_qemu_ga() {
        let cfg = guest_agent_channel();
        let wrapped = format!("<domain><devices>  {}\n</devices></domain>", build_channel(&cfg));
        let parsed = parse_channels(&wrapped).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].target_name.as_deref(), Some("org.qemu.guest_agent.0"));
        assert_eq!(parsed[0].target_type, "virtio");
        assert!(matches!(parsed[0].source, CharDeviceType::Unix { mode: UnixMode::Bind, .. }));
    }

    #[test]
    fn parallel_round_trip() {
        let cfg = ParallelConfig {
            source: CharDeviceType::Pty,
            target_port: Some(0),
        };
        let wrapped = format!("<domain><devices>  {}\n</devices></domain>", build_parallel(&cfg));
        let parsed = parse_parallels(&wrapped).unwrap();
        assert_eq!(parsed[0], cfg);
    }

    #[test]
    fn qemu_ga_preset_is_correct() {
        let ga = guest_agent_channel();
        assert_eq!(ga.target_type, "virtio");
        assert_eq!(ga.target_name.as_deref(), Some("org.qemu.guest_agent.0"));
        match ga.source {
            CharDeviceType::Unix { mode: UnixMode::Bind, .. } => {},
            _ => panic!("qemu-ga must be unix bind"),
        }
    }

    #[test]
    fn vdagent_preset_is_correct() {
        let v = spice_vdagent_channel();
        assert_eq!(v.target_type, "virtio");
        assert_eq!(v.target_name.as_deref(), Some("com.redhat.spice.0"));
        assert_eq!(v.source, CharDeviceType::Spicevmc);
    }

    #[test]
    fn multiple_channels_preserve_order() {
        let xml = r#"<domain><devices>
            <channel type='unix'><source mode='bind' path='/a'/><target type='virtio' name='a.0'/></channel>
            <channel type='spicevmc'><target type='virtio' name='b.0'/></channel>
            <channel type='unix'><source mode='bind' path='/c'/><target type='virtio' name='c.0'/></channel>
        </devices></domain>"#;
        let ch = parse_channels(xml).unwrap();
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[0].target_name.as_deref(), Some("a.0"));
        assert_eq!(ch[1].target_name.as_deref(), Some("b.0"));
        assert_eq!(ch[2].target_name.as_deref(), Some("c.0"));
    }

    #[test]
    fn escape_injection_in_host_and_path() {
        let cfg = SerialConfig {
            source: CharDeviceType::Tcp {
                host: "evil'><!--".into(),
                port: 4000,
                mode: TcpMode::Connect,
                protocol: TcpProtocol::Raw,
            },
            target_type: "isa-serial".into(),
            target_port: Some(0),
        };
        let xml = build_serial(&cfg);
        assert!(!xml.contains("evil'><!--"));
        assert!(xml.contains("&apos;"));
        assert!(xml.contains("&lt;"));

        let cfg2 = ChannelConfig {
            source: CharDeviceType::Unix {
                path: "/tmp/x'>&<y".into(),
                mode: UnixMode::Bind,
            },
            target_type: "virtio".into(),
            target_name: Some("weird'>".into()),
        };
        let xml2 = build_channel(&cfg2);
        assert!(xml2.contains("&amp;"));
        assert!(xml2.contains("&apos;"));
        assert!(!xml2.contains("'>&<"));
    }

    #[test]
    fn unknown_source_type_skipped_gracefully() {
        let xml = r#"<domain><devices>
            <serial type='this-does-not-exist'><target type='isa-serial' port='0'/></serial>
            <serial type='pty'><target type='isa-serial' port='1'/></serial>
        </devices></domain>"#;
        let s = parse_serials(xml).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].target_port, Some(1));
    }

    #[test]
    fn apply_add_channel_appends_before_devices_close() {
        let base = r#"<domain><devices>
    <serial type='pty'><target type='isa-serial' port='0'/></serial>
  </devices></domain>"#;
        let new_xml = apply_add_channel(base, &guest_agent_channel()).unwrap();
        let ch = parse_channels(&new_xml).unwrap();
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].target_name.as_deref(), Some("org.qemu.guest_agent.0"));
        // serial still present
        let s = parse_serials(&new_xml).unwrap();
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn apply_remove_channel_by_name() {
        let s = apply_add_channel("<domain><devices></devices></domain>", &guest_agent_channel()).unwrap();
        let s2 = apply_add_channel(&s, &spice_vdagent_channel()).unwrap();
        assert_eq!(parse_channels(&s2).unwrap().len(), 2);
        let s3 = apply_remove_channel(&s2, "org.qemu.guest_agent.0").unwrap();
        let after = parse_channels(&s3).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].target_name.as_deref(), Some("com.redhat.spice.0"));
    }

    #[test]
    fn apply_remove_serial_by_port() {
        let base = r#"<domain><devices>
    <serial type='pty'><target type='isa-serial' port='0'/></serial>
    <serial type='pty'><target type='isa-serial' port='1'/></serial>
  </devices></domain>"#;
        let out = apply_remove_serial(base, 0).unwrap();
        let s = parse_serials(&out).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].target_port, Some(1));
    }

    #[test]
    fn snapshot_bundles_everything() {
        let serials = parse_serials(FEDORA_XML).unwrap();
        let consoles = parse_consoles(FEDORA_XML).unwrap();
        let channels = parse_channels(FEDORA_XML).unwrap();
        let parallels = parse_parallels(FEDORA_XML).unwrap();
        assert_eq!(serials.len(), 1);
        assert_eq!(consoles.len(), 1);
        assert_eq!(channels.len(), 2);
        assert_eq!(parallels.len(), 0);
    }
}
