//! Graphics / video / sound / input device editor (Round D).
//!
//! Parses and patches the display-related device entries in a domain XML:
//! `<graphics>`, `<video>`, `<sound>`, `<input>`.
//!
//! Most domains contain a single `<graphics>` (the primary display),
//! a single primary `<video>`, a single `<sound>`, and 1-3 `<input>`
//! entries. Our edit surface mirrors that: `apply_replace_*` replaces
//! the first matching device, preserving everything else in the XML
//! byte-for-byte (quick-xml streaming copy).
//!
//! NOTE on graphics password: libvirt only returns `<graphics passwd>`
//! if you call `virDomainGetXMLDesc` with `VIR_DOMAIN_XML_SECURE`. Our
//! `LibvirtConnection::get_domain_xml` does NOT request that flag by
//! default, so `GraphicsConfig::passwd` will usually be empty when
//! round-tripping through `get_display_config`. The serializer still
//! writes the field if present.

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

// ─────────────────────────── data types ────────────────────────────

/// `<graphics>` element.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GraphicsConfig {
    /// vnc, spice, rdp, sdl, dbus, egl-headless, none.
    #[serde(rename = "type")]
    pub r#type: String,
    /// listen address — "127.0.0.1" / "0.0.0.0" / etc. Mutually exclusive
    /// with `listen_socket`.
    pub listen: Option<String>,
    /// When the VM uses `<listen type='socket' socket='/path'/>`.
    pub listen_socket: Option<String>,
    /// -1 = autoport.
    pub port: Option<i32>,
    pub autoport: bool,
    pub tls_port: Option<i32>,
    pub passwd: Option<String>,
    pub passwd_valid_to: Option<String>,
    pub keymap: Option<String>,
    /// SPICE default-mode: "any" / "secure" / "insecure".
    pub default_mode: Option<String>,
    /// `<gl enable='yes'/>` in the graphics block.
    pub gl_accel: bool,
    /// `<gl rendernode='/dev/dri/renderD128'/>`.
    pub rendernode: Option<String>,
    /// SPICE `<image compression='...'/>`.
    pub image_compression: Option<String>,
    /// SPICE `<streaming mode='...'/>`.
    pub streaming_mode: Option<String>,
}

/// `<video>` element.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VideoConfig {
    /// vga, cirrus, qxl, virtio, bochs, ramfb, vmvga, none.
    pub model: String,
    /// Total video RAM in KiB (`<model vram='...'/>`).
    pub vram: Option<u32>,
    pub ram: Option<u32>,
    pub vgamem: Option<u32>,
    pub heads: Option<u32>,
    pub primary: bool,
    /// virtio `blob='on'` (direct host-mapped framebuffer).
    pub blob: Option<bool>,
    /// virtio `accel3d='yes'`.
    pub accel3d: bool,
}

/// `<sound>` element.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SoundConfig {
    /// ich9, ich7, ich6, ac97, hda, es1370, sb16, usb.
    pub model: String,
    pub codecs: Vec<SoundCodec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SoundCodec {
    /// duplex, micro, output.
    #[serde(rename = "type")]
    pub r#type: String,
}

/// `<input>` element.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct InputConfig {
    /// mouse, keyboard, tablet, passthrough, evdev.
    #[serde(rename = "type")]
    pub r#type: String,
    /// usb, virtio, ps2, xen.
    pub bus: Option<String>,
}

/// Bundle of display devices — what the UI pulls in one shot.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DisplayConfig {
    pub graphics: Vec<GraphicsConfig>,
    pub video: Vec<VideoConfig>,
    pub sound: Vec<SoundConfig>,
    pub input: Vec<InputConfig>,
}

/// Optional patch covering each subsection. Fields left None are
/// untouched. When a subsection is Some it REPLACES the first existing
/// device of its kind (or appends if none). `inputs` replaces the
/// entire input list.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayPatch {
    pub graphics: Option<GraphicsConfig>,
    pub video: Option<VideoConfig>,
    pub sound: Option<SoundConfig>,
    pub inputs: Option<Vec<InputConfig>>,
}

// ─────────────────────────── parse ─────────────────────────────────

/// Parse all `<graphics>` entries in a domain XML.
pub fn parse_graphics(xml: &str) -> Result<Vec<GraphicsConfig>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out: Vec<GraphicsConfig> = Vec::new();
    let mut current: Option<GraphicsConfig> = None;
    let mut buf = Vec::new();

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: e.to_string(),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if name_eq(&e, "graphics") => {
                current = Some(parse_graphics_attrs(&e));
            }
            Ok(Event::Empty(e)) if name_eq(&e, "graphics") => {
                out.push(parse_graphics_attrs(&e));
            }
            Ok(Event::End(e)) if name_eq_end(&e, "graphics") => {
                if let Some(g) = current.take() {
                    out.push(g);
                }
            }
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if let Some(ref mut g) = current {
                    let n = utf8_name(e);
                    let a = attrs(e);
                    let attr = |k: &str| {
                        a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
                    };
                    match n.as_str() {
                        "listen" => match attr("type").as_deref() {
                            Some("socket") => {
                                g.listen_socket = attr("socket");
                            }
                            _ => {
                                if let Some(ad) = attr("address") {
                                    g.listen = Some(ad);
                                }
                            }
                        },
                        "gl" => {
                            if attr("enable").as_deref() == Some("yes") {
                                g.gl_accel = true;
                            }
                            if let Some(rn) = attr("rendernode") {
                                g.rendernode = Some(rn);
                            }
                        }
                        "image" => {
                            g.image_compression = attr("compression");
                        }
                        "streaming" => {
                            g.streaming_mode = attr("mode");
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn parse_graphics_attrs(e: &BytesStart) -> GraphicsConfig {
    let a = attrs(e);
    let attr = |k: &str| a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    GraphicsConfig {
        r#type: attr("type").unwrap_or_default(),
        listen: attr("listen"),
        listen_socket: None,
        port: attr("port").and_then(|s| s.parse().ok()),
        autoport: attr("autoport").as_deref() == Some("yes"),
        tls_port: attr("tlsPort").and_then(|s| s.parse().ok()),
        passwd: attr("passwd"),
        passwd_valid_to: attr("passwdValidTo"),
        keymap: attr("keymap"),
        default_mode: attr("defaultMode"),
        gl_accel: false,
        rendernode: None,
        image_compression: None,
        streaming_mode: None,
    }
}

/// Parse all `<video>` entries.
pub fn parse_video(xml: &str) -> Result<Vec<VideoConfig>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out: Vec<VideoConfig> = Vec::new();
    let mut current: Option<VideoConfig> = None;
    let mut buf = Vec::new();

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: e.to_string(),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if name_eq(&e, "video") => {
                current = Some(VideoConfig::default());
            }
            Ok(Event::Empty(e)) if name_eq(&e, "video") => {
                out.push(VideoConfig::default());
            }
            Ok(Event::End(e)) if name_eq_end(&e, "video") => {
                if let Some(v) = current.take() {
                    out.push(v);
                }
            }
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if let Some(ref mut v) = current {
                    let n = utf8_name(e);
                    let a = attrs(e);
                    let attr = |k: &str| {
                        a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
                    };
                    match n.as_str() {
                        "model" => {
                            if let Some(t) = attr("type") {
                                v.model = t;
                            }
                            v.vram = attr("vram").and_then(|s| s.parse().ok());
                            v.ram = attr("ram").and_then(|s| s.parse().ok());
                            v.vgamem = attr("vgamem").and_then(|s| s.parse().ok());
                            v.heads = attr("heads").and_then(|s| s.parse().ok());
                            if attr("primary").as_deref() == Some("yes") {
                                v.primary = true;
                            }
                            v.blob = attr("blob").map(|s| s == "on" || s == "yes");
                        }
                        "acceleration" => {
                            if attr("accel3d").as_deref() == Some("yes") {
                                v.accel3d = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// Parse all `<sound>` entries.
pub fn parse_sound(xml: &str) -> Result<Vec<SoundConfig>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out: Vec<SoundConfig> = Vec::new();
    let mut current: Option<SoundConfig> = None;
    let mut buf = Vec::new();

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: e.to_string(),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if name_eq(&e, "sound") => {
                let a = attrs(&e);
                let model = a
                    .iter()
                    .find(|(k, _)| k == "model")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                current = Some(SoundConfig { model, codecs: Vec::new() });
            }
            Ok(Event::Empty(e)) if name_eq(&e, "sound") => {
                let a = attrs(&e);
                let model = a
                    .iter()
                    .find(|(k, _)| k == "model")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                out.push(SoundConfig { model, codecs: Vec::new() });
            }
            Ok(Event::End(e)) if name_eq_end(&e, "sound") => {
                if let Some(s) = current.take() {
                    out.push(s);
                }
            }
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if let Some(ref mut s) = current {
                    if name_eq(e, "codec") {
                        let a = attrs(e);
                        let t = a
                            .iter()
                            .find(|(k, _)| k == "type")
                            .map(|(_, v)| v.clone())
                            .unwrap_or_default();
                        s.codecs.push(SoundCodec { r#type: t });
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// Parse all `<input>` entries.
pub fn parse_input(xml: &str) -> Result<Vec<InputConfig>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out: Vec<InputConfig> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: e.to_string(),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) if name_eq(e, "input") => {
                let a = attrs(e);
                let t = a
                    .iter()
                    .find(|(k, _)| k == "type")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                let bus = a.iter().find(|(k, _)| k == "bus").map(|(_, v)| v.clone());
                out.push(InputConfig { r#type: t, bus });
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

// ─────────────────────────── build ─────────────────────────────────

pub fn build_graphics_xml(g: &GraphicsConfig) -> String {
    let mut s = String::from("<graphics");
    s.push_str(&format!(" type='{}'", escape_xml(&g.r#type)));
    if let Some(p) = g.port {
        s.push_str(&format!(" port='{}'", p));
    }
    if g.autoport {
        s.push_str(" autoport='yes'");
    }
    if let Some(p) = g.tls_port {
        s.push_str(&format!(" tlsPort='{}'", p));
    }
    if let Some(ref addr) = g.listen {
        s.push_str(&format!(" listen='{}'", escape_xml(addr)));
    }
    if let Some(ref pw) = g.passwd {
        s.push_str(&format!(" passwd='{}'", escape_xml(pw)));
    }
    if let Some(ref v) = g.passwd_valid_to {
        s.push_str(&format!(" passwdValidTo='{}'", escape_xml(v)));
    }
    if let Some(ref k) = g.keymap {
        s.push_str(&format!(" keymap='{}'", escape_xml(k)));
    }
    if let Some(ref d) = g.default_mode {
        s.push_str(&format!(" defaultMode='{}'", escape_xml(d)));
    }

    let has_children = g.listen.is_some()
        || g.listen_socket.is_some()
        || g.gl_accel
        || g.rendernode.is_some()
        || g.image_compression.is_some()
        || g.streaming_mode.is_some();

    if !has_children {
        s.push_str("/>");
        return s;
    }

    s.push_str(">\n");
    if let Some(ref sock) = g.listen_socket {
        s.push_str(&format!(
            "        <listen type='socket' socket='{}'/>\n",
            escape_xml(sock)
        ));
    } else if let Some(ref addr) = g.listen {
        s.push_str(&format!(
            "        <listen type='address' address='{}'/>\n",
            escape_xml(addr)
        ));
    }
    if g.gl_accel || g.rendernode.is_some() {
        let mut gl = String::from("        <gl");
        if g.gl_accel {
            gl.push_str(" enable='yes'");
        }
        if let Some(ref rn) = g.rendernode {
            gl.push_str(&format!(" rendernode='{}'", escape_xml(rn)));
        }
        gl.push_str("/>\n");
        s.push_str(&gl);
    }
    if let Some(ref c) = g.image_compression {
        s.push_str(&format!(
            "        <image compression='{}'/>\n",
            escape_xml(c)
        ));
    }
    if let Some(ref m) = g.streaming_mode {
        s.push_str(&format!(
            "        <streaming mode='{}'/>\n",
            escape_xml(m)
        ));
    }
    s.push_str("      </graphics>");
    s
}

pub fn build_video_xml(v: &VideoConfig) -> String {
    let mut s = String::from("<video>\n        <model");
    s.push_str(&format!(" type='{}'", escape_xml(&v.model)));
    if let Some(x) = v.vram {
        s.push_str(&format!(" vram='{}'", x));
    }
    if let Some(x) = v.ram {
        s.push_str(&format!(" ram='{}'", x));
    }
    if let Some(x) = v.vgamem {
        s.push_str(&format!(" vgamem='{}'", x));
    }
    if let Some(x) = v.heads {
        s.push_str(&format!(" heads='{}'", x));
    }
    if v.primary {
        s.push_str(" primary='yes'");
    }
    if let Some(b) = v.blob {
        s.push_str(&format!(" blob='{}'", if b { "on" } else { "off" }));
    }

    if v.accel3d {
        s.push_str(">\n          <acceleration accel3d='yes'/>\n        </model>\n");
    } else {
        s.push_str("/>\n");
    }
    s.push_str("      </video>");
    s
}

pub fn build_sound_xml(s: &SoundConfig) -> String {
    let mut out = format!("<sound model='{}'", escape_xml(&s.model));
    if s.codecs.is_empty() {
        out.push_str("/>");
        return out;
    }
    out.push_str(">\n");
    for c in &s.codecs {
        out.push_str(&format!(
            "        <codec type='{}'/>\n",
            escape_xml(&c.r#type)
        ));
    }
    out.push_str("      </sound>");
    out
}

pub fn build_input_xml(i: &InputConfig) -> String {
    let mut s = format!("<input type='{}'", escape_xml(&i.r#type));
    if let Some(ref b) = i.bus {
        s.push_str(&format!(" bus='{}'", escape_xml(b)));
    }
    s.push_str("/>");
    s
}

// ─────────────────────────── apply ─────────────────────────────────

/// Replace the first `<graphics>` element with the new config. If no
/// `<graphics>` exists, the new one is inserted before `</devices>`.
/// Everything else in the XML (video, sound, input, disks, etc.) is
/// preserved verbatim.
pub fn apply_replace_graphics(
    xml: &str,
    g: &GraphicsConfig,
) -> Result<String, VirtManagerError> {
    replace_first_or_insert(xml, "graphics", &build_graphics_xml(g))
}

pub fn apply_replace_video(xml: &str, v: &VideoConfig) -> Result<String, VirtManagerError> {
    replace_first_or_insert(xml, "video", &build_video_xml(v))
}

pub fn apply_replace_sound(xml: &str, s: &SoundConfig) -> Result<String, VirtManagerError> {
    replace_first_or_insert(xml, "sound", &build_sound_xml(s))
}

/// Replace the first `<input>`. For bulk list replacement (more common
/// for input), use `apply_replace_inputs`.
pub fn apply_replace_input(xml: &str, i: &InputConfig) -> Result<String, VirtManagerError> {
    replace_first_or_insert(xml, "input", &build_input_xml(i))
}

/// Replace ALL `<input>` entries with the given list. Useful when the
/// user wants "tablet + keyboard" instead of the default PS/2 mouse.
pub fn apply_replace_inputs(
    xml: &str,
    inputs: &[InputConfig],
) -> Result<String, VirtManagerError> {
    let stripped = strip_all(xml, "input")?;
    let body = inputs
        .iter()
        .map(|i| format!("    {}\n", build_input_xml(i)))
        .collect::<String>();
    if body.is_empty() {
        return Ok(stripped);
    }
    if let Some(idx) = stripped.rfind("</devices>") {
        let mut out = String::with_capacity(stripped.len() + body.len());
        out.push_str(&stripped[..idx]);
        out.push_str(&body);
        out.push_str(&stripped[idx..]);
        Ok(out)
    } else {
        Err(VirtManagerError::XmlParsingFailed {
            reason: "no </devices> to insert inputs into".into(),
        })
    }
}

/// Streaming replace of the FIRST element named `tag`. If the element
/// is not found, insert `new_content` before `</devices>`.
fn replace_first_or_insert(
    xml: &str,
    tag: &str,
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
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: e.to_string(),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if name_eq(&e, tag) => {
                if start_byte.is_none() {
                    start_byte = Some(pos_before);
                }
                depth += 1;
            }
            Ok(Event::Empty(e)) if name_eq(&e, tag) => {
                if start_byte.is_none() {
                    start_byte = Some(pos_before);
                    end_byte = Some(r.buffer_position() as usize);
                    break;
                }
            }
            Ok(Event::End(e)) if name_eq_end(&e, tag) => {
                if start_byte.is_some() {
                    depth -= 1;
                    if depth == 0 {
                        end_byte = Some(r.buffer_position() as usize);
                        break;
                    }
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
            if let Some(idx) = xml.rfind("</devices>") {
                let mut out = String::with_capacity(xml.len() + new_content.len() + 8);
                out.push_str(&xml[..idx]);
                out.push_str("    ");
                out.push_str(new_content);
                out.push('\n');
                out.push_str(&xml[idx..]);
                Ok(out)
            } else {
                Err(VirtManagerError::XmlParsingFailed {
                    reason: format!("no </devices> to insert <{tag}> into"),
                })
            }
        }
    }
}

/// Remove every top-level occurrence of `<tag>...</tag>` (or empty tag)
/// from the XML. Returns the cleaned XML.
fn strip_all(xml: &str, tag: &str) -> Result<String, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut depth: i32 = 0;
    let mut cur_start: Option<usize> = None;

    loop {
        let pos_before = r.buffer_position() as usize;
        match r.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: e.to_string(),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if name_eq(&e, tag) => {
                if cur_start.is_none() {
                    cur_start = Some(pos_before);
                }
                depth += 1;
            }
            Ok(Event::Empty(e)) if name_eq(&e, tag) => {
                if cur_start.is_none() {
                    ranges.push((pos_before, r.buffer_position() as usize));
                }
            }
            Ok(Event::End(e)) if name_eq_end(&e, tag) => {
                if cur_start.is_some() {
                    depth -= 1;
                    if depth == 0 {
                        let s = cur_start.take().unwrap();
                        ranges.push((s, r.buffer_position() as usize));
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    if ranges.is_empty() {
        return Ok(xml.to_string());
    }

    let mut out = String::with_capacity(xml.len());
    let mut cursor = 0usize;
    for (s, e) in ranges {
        let mut line_start = s;
        while line_start > cursor {
            let b = xml.as_bytes()[line_start - 1];
            if b == b' ' || b == b'\t' {
                line_start -= 1;
            } else {
                break;
            }
        }
        out.push_str(&xml[cursor..line_start]);
        let mut after = e;
        if xml.as_bytes().get(after).copied() == Some(b'\n') {
            after += 1;
        }
        cursor = after;
    }
    out.push_str(&xml[cursor..]);
    Ok(out)
}

// ─────────────────────────── helpers ───────────────────────────────

fn name_eq(e: &BytesStart, name: &str) -> bool {
    e.name().as_ref() == name.as_bytes()
}

fn name_eq_end(e: &BytesEnd, name: &str) -> bool {
    e.name().as_ref() == name.as_bytes()
}

fn utf8_name(e: &BytesStart) -> String {
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

// ─────────────────────────── tests ─────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<domain type='kvm'>
  <name>t</name>
  <devices>
    <emulator>/usr/bin/qemu</emulator>
    <input type='tablet' bus='usb'>
      <alias name='input0'/>
    </input>
    <input type='mouse' bus='ps2'/>
    <input type='keyboard' bus='ps2'/>
    <graphics type='spice' port='5900' autoport='yes' listen='127.0.0.1'>
      <listen type='address' address='127.0.0.1'/>
      <image compression='off'/>
    </graphics>
    <sound model='ich9'>
      <codec type='duplex'/>
    </sound>
    <video>
      <model type='virtio' heads='1' primary='yes'/>
    </video>
  </devices>
</domain>
"#;

    #[test]
    fn parse_graphics_spice() {
        let g = parse_graphics(SAMPLE).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].r#type, "spice");
        assert_eq!(g[0].port, Some(5900));
        assert!(g[0].autoport);
        assert_eq!(g[0].listen.as_deref(), Some("127.0.0.1"));
        assert_eq!(g[0].image_compression.as_deref(), Some("off"));
    }

    #[test]
    fn parse_video_virtio() {
        let v = parse_video(SAMPLE).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].model, "virtio");
        assert_eq!(v[0].heads, Some(1));
        assert!(v[0].primary);
    }

    #[test]
    fn parse_sound_ich9_with_codec() {
        let s = parse_sound(SAMPLE).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].model, "ich9");
        assert_eq!(s[0].codecs.len(), 1);
        assert_eq!(s[0].codecs[0].r#type, "duplex");
    }

    #[test]
    fn parse_input_three_devices() {
        let i = parse_input(SAMPLE).unwrap();
        assert_eq!(i.len(), 3);
        assert_eq!(i[0].r#type, "tablet");
        assert_eq!(i[0].bus.as_deref(), Some("usb"));
        assert_eq!(i[1].r#type, "mouse");
        assert_eq!(i[2].r#type, "keyboard");
    }

    #[test]
    fn graphics_round_trip_preserves_fields() {
        let g = GraphicsConfig {
            r#type: "vnc".into(),
            listen: Some("0.0.0.0".into()),
            port: Some(-1),
            autoport: true,
            keymap: Some("en-us".into()),
            passwd: Some("hunter2".into()),
            ..Default::default()
        };
        let xml = format!("<domain><devices>{}</devices></domain>", build_graphics_xml(&g));
        let parsed = parse_graphics(&xml).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "vnc");
        assert_eq!(parsed[0].listen.as_deref(), Some("0.0.0.0"));
        assert_eq!(parsed[0].port, Some(-1));
        assert!(parsed[0].autoport);
        assert_eq!(parsed[0].keymap.as_deref(), Some("en-us"));
        assert_eq!(parsed[0].passwd.as_deref(), Some("hunter2"));
    }

    #[test]
    fn video_round_trip_common_models() {
        for model in ["vga", "cirrus", "qxl", "virtio", "bochs", "ramfb"] {
            let v = VideoConfig {
                model: model.into(),
                vram: Some(16384),
                heads: Some(1),
                primary: true,
                ..Default::default()
            };
            let xml = format!("<domain><devices>{}</devices></domain>", build_video_xml(&v));
            let parsed = parse_video(&xml).unwrap();
            assert_eq!(parsed.len(), 1, "{model}");
            assert_eq!(parsed[0].model, model);
            assert_eq!(parsed[0].vram, Some(16384));
            assert!(parsed[0].primary);
        }
    }

    #[test]
    fn sound_round_trip_multi_codec() {
        let s = SoundConfig {
            model: "hda".into(),
            codecs: vec![
                SoundCodec { r#type: "duplex".into() },
                SoundCodec { r#type: "micro".into() },
            ],
        };
        let xml = format!("<domain><devices>{}</devices></domain>", build_sound_xml(&s));
        let parsed = parse_sound(&xml).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].codecs.len(), 2);
        assert_eq!(parsed[0].codecs[0].r#type, "duplex");
        assert_eq!(parsed[0].codecs[1].r#type, "micro");
    }

    #[test]
    fn input_round_trip_tablet_and_keyboard() {
        let inputs = vec![
            InputConfig { r#type: "tablet".into(), bus: Some("usb".into()) },
            InputConfig { r#type: "keyboard".into(), bus: Some("ps2".into()) },
        ];
        let body: String = inputs.iter().map(build_input_xml).collect::<Vec<_>>().join("");
        let xml = format!("<domain><devices>{}</devices></domain>", body);
        let parsed = parse_input(&xml).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].r#type, "tablet");
        assert_eq!(parsed[1].bus.as_deref(), Some("ps2"));
    }

    #[test]
    fn replace_graphics_preserves_video_and_sound() {
        let new_g = GraphicsConfig {
            r#type: "vnc".into(),
            listen: Some("0.0.0.0".into()),
            port: Some(-1),
            autoport: true,
            ..Default::default()
        };
        let out = apply_replace_graphics(SAMPLE, &new_g).unwrap();
        assert!(out.contains("type='vnc'"));
        assert!(out.contains("<video>"));
        assert!(out.contains("type='virtio'"));
        assert!(out.contains("<sound model='ich9'"));
        let g = parse_graphics(&out).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].r#type, "vnc");
    }

    #[test]
    fn replace_video_preserves_graphics_and_input() {
        let new_v = VideoConfig {
            model: "cirrus".into(),
            vram: Some(9216),
            heads: Some(1),
            primary: true,
            ..Default::default()
        };
        let out = apply_replace_video(SAMPLE, &new_v).unwrap();
        let v = parse_video(&out).unwrap();
        assert_eq!(v[0].model, "cirrus");
        let g = parse_graphics(&out).unwrap();
        assert_eq!(g[0].r#type, "spice");
        let i = parse_input(&out).unwrap();
        assert_eq!(i.len(), 3);
    }

    #[test]
    fn replace_sound_preserves_rest() {
        let new_s = SoundConfig { model: "hda".into(), codecs: vec![] };
        let out = apply_replace_sound(SAMPLE, &new_s).unwrap();
        assert!(out.contains("<sound model='hda'"));
        let v = parse_video(&out).unwrap();
        assert_eq!(v[0].model, "virtio");
    }

    #[test]
    fn replace_all_inputs_tablet_plus_keyboard() {
        let new_inputs = vec![
            InputConfig { r#type: "tablet".into(), bus: Some("usb".into()) },
            InputConfig { r#type: "keyboard".into(), bus: Some("virtio".into()) },
        ];
        let out = apply_replace_inputs(SAMPLE, &new_inputs).unwrap();
        let i = parse_input(&out).unwrap();
        assert_eq!(i.len(), 2);
        assert_eq!(i[0].r#type, "tablet");
        assert_eq!(i[1].bus.as_deref(), Some("virtio"));
        assert!(out.contains("<graphics"));
        assert!(out.contains("<video>"));
    }

    #[test]
    fn injection_safe_listen_and_keymap_and_rendernode() {
        let g = GraphicsConfig {
            r#type: "vnc".into(),
            listen: Some("127.0.0.1' evil='yes".into()),
            keymap: Some("en-us<script>".into()),
            rendernode: Some("/dev/dri/render'oops".into()),
            gl_accel: true,
            ..Default::default()
        };
        let out = build_graphics_xml(&g);
        assert!(!out.contains("evil='yes"));
        assert!(!out.contains("<script>"));
        assert!(!out.contains("render'oops"));
        assert!(out.contains("&apos;") || out.contains("&quot;"));
        assert!(out.contains("&lt;script&gt;"));
    }

    #[test]
    fn graphics_socket_listen_round_trip() {
        let xml = r#"<domain><devices>
            <graphics type='spice' autoport='yes'>
              <listen type='socket' socket='/var/run/spice.sock'/>
            </graphics>
        </devices></domain>"#;
        let g = parse_graphics(xml).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].listen_socket.as_deref(), Some("/var/run/spice.sock"));
        assert!(g[0].listen.is_none());

        let built = build_graphics_xml(&g[0]);
        assert!(built.contains("type='socket'"));
        assert!(built.contains("socket='/var/run/spice.sock'"));
    }

    #[test]
    fn graphics_gl_accel_round_trip() {
        let g = GraphicsConfig {
            r#type: "spice".into(),
            listen: Some("127.0.0.1".into()),
            autoport: true,
            gl_accel: true,
            rendernode: Some("/dev/dri/renderD128".into()),
            ..Default::default()
        };
        let xml = format!("<domain><devices>{}</devices></domain>", build_graphics_xml(&g));
        let parsed = parse_graphics(&xml).unwrap();
        assert!(parsed[0].gl_accel);
        assert_eq!(parsed[0].rendernode.as_deref(), Some("/dev/dri/renderD128"));
    }

    #[test]
    fn video_accel3d_round_trip() {
        let v = VideoConfig {
            model: "virtio".into(),
            heads: Some(1),
            primary: true,
            accel3d: true,
            ..Default::default()
        };
        let xml = format!("<domain><devices>{}</devices></domain>", build_video_xml(&v));
        let parsed = parse_video(&xml).unwrap();
        assert!(parsed[0].accel3d);
    }

    #[test]
    fn insert_graphics_when_missing() {
        let xml = r#"<domain><devices><emulator>/q</emulator></devices></domain>"#;
        let g = GraphicsConfig { r#type: "vnc".into(), autoport: true, ..Default::default() };
        let out = apply_replace_graphics(xml, &g).unwrap();
        let parsed = parse_graphics(&out).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "vnc");
    }
}
