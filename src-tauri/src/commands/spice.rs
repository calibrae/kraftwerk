//! SPICE console commands.
//!
//! `open_spice` spawns an SSH tunnel + capsaicin client for a domain.
//! Display events are forwarded to the webview as `spice:event` Tauri
//! events. Input events are pushed back via `spice_input`.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use capsaicin_client::{
    ClientEvent, DisplayEvent, InputEvent, Rect, RegionPixels, SurfaceFormat,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::app_state::AppState;
use crate::libvirt::spice_proxy::{self, SpiceSession};
use crate::libvirt::vnc_proxy;
use crate::models::error::VirtManagerError;

// ── Frontend DTOs ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RectDto {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
}

impl From<Rect> for RectDto {
    fn from(r: Rect) -> Self {
        Self {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
            width: r.right - r.left,
            height: r.bottom - r.top,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PixelsDto {
    SolidColor { argb: u32 },
    /// Base64-encoded pixels to reduce JSON-array blowup.
    Raw { data_b64: String, stride: u32 },
}

impl PixelsDto {
    fn from_region(p: RegionPixels) -> Self {
        match p {
            RegionPixels::SolidColor(c) => Self::SolidColor { argb: c },
            RegionPixels::Raw { data, stride } => Self::Raw {
                data_b64: BASE64.encode(&data),
                stride,
            },
        }
    }
}

fn surface_format_str(f: SurfaceFormat) -> &'static str {
    match f {
        SurfaceFormat::Xrgb8888 => "xrgb8888",
        SurfaceFormat::Argb8888 => "argb8888",
        SurfaceFormat::Rgb565 => "rgb565",
        SurfaceFormat::Rgb555 => "rgb555",
        SurfaceFormat::A8 => "a8",
        SurfaceFormat::A1 => "a1",
        SurfaceFormat::Unknown(_) => "unknown",
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpiceEventDto {
    SurfaceCreated {
        id: u32,
        width: u32,
        height: u32,
        format: &'static str,
        primary: bool,
    },
    SurfaceDestroyed {
        id: u32,
    },
    Region {
        #[serde(rename = "surfaceId")]
        surface_id: u32,
        rect: RectDto,
        pixels: PixelsDto,
        format: &'static str,
    },
    #[serde(rename_all = "camelCase")]
    CopyRect {
        surface_id: u32,
        src_x: i32,
        src_y: i32,
        dest_rect: RectDto,
    },
    StreamFrame {
        stream_id: u32,
        dest_rect: RectDto,
        pixels: PixelsDto,
    },
    StreamCreated {
        stream_id: u32,
        surface_id: u32,
        codec: String,
        dest: RectDto,
        src_width: u32,
        src_height: u32,
    },
    StreamDestroyed {
        stream_id: u32,
    },
    Mark,
    Reset,
    Mode {
        width: u32,
        height: u32,
    },
    Closed {
        reason: Option<String>,
    },
}

fn to_dto(evt: ClientEvent) -> Option<SpiceEventDto> {
    Some(match evt {
        ClientEvent::Display(d) => match d {
            DisplayEvent::SurfaceCreated { id, width, height, format, primary } => {
                SpiceEventDto::SurfaceCreated {
                    id, width, height,
                    format: surface_format_str(format),
                    primary,
                }
            }
            DisplayEvent::SurfaceDestroyed { id } => SpiceEventDto::SurfaceDestroyed { id },
            DisplayEvent::Region { surface_id, rect, pixels, surface_format } => {
                SpiceEventDto::Region {
                    surface_id,
                    rect: rect.into(),
                    pixels: PixelsDto::from_region(pixels),
                    format: surface_format_str(surface_format),
                }
            }
            DisplayEvent::CopyRect { surface_id, src_x, src_y, dest_rect } => {
                SpiceEventDto::CopyRect { surface_id, src_x, src_y, dest_rect: dest_rect.into() }
            }
            DisplayEvent::StreamCreated { stream_id, surface_id, codec, dest, src_width, src_height } => {
                SpiceEventDto::StreamCreated {
                    stream_id, surface_id,
                    codec: format!("{codec:?}"),
                    dest: dest.into(),
                    src_width, src_height,
                }
            }
            DisplayEvent::StreamFrame { stream_id, dest_rect, pixels, .. } => {
                SpiceEventDto::StreamFrame {
                    stream_id,
                    dest_rect: dest_rect.into(),
                    pixels: PixelsDto::from_region(pixels),
                }
            }
            DisplayEvent::StreamDestroyed { stream_id } => SpiceEventDto::StreamDestroyed { stream_id },
            DisplayEvent::Mark => SpiceEventDto::Mark,
            DisplayEvent::Reset => SpiceEventDto::Reset,
            DisplayEvent::Mode { width, height, .. } => SpiceEventDto::Mode { width, height },
            DisplayEvent::MonitorsConfig { .. } | DisplayEvent::UnhandledDraw { .. } => return None,
        },
        ClientEvent::Closed(e) => SpiceEventDto::Closed {
            reason: e.map(|e| e.to_string()),
        },
    })
}

// ── Input DTO ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEventDto {
    KeyDown { scancode: u32 },
    KeyUp { scancode: u32 },
    MousePosition {
        x: u32,
        y: u32,
        #[serde(default)]
        buttons: u32,
    },
    MouseMotion {
        dx: i32,
        dy: i32,
        #[serde(default)]
        buttons: u32,
    },
    MousePress { button: u8, #[serde(default)] buttons: u32 },
    MouseRelease { button: u8, #[serde(default)] buttons: u32 },
}

/// SPICE signals key release by setting bit 0x80 on the *low byte* of
/// the scancode. For 0xE0-prefixed extended keys (arrows, right-ctrl,
/// etc.) the high byte must be preserved — a naive `scancode | 0x80`
/// corrupts scancodes whose high byte intersects 0x80.
///
/// Example: Right arrow = 0xE04D down, 0xE0CD up.
fn release_scancode(scancode: u32) -> u32 {
    (scancode & 0xFFFF_FF00) | ((scancode & 0xFF) | 0x80)
}

impl From<InputEventDto> for InputEvent {
    fn from(v: InputEventDto) -> Self {
        match v {
            InputEventDto::KeyDown { scancode } => InputEvent::KeyDown(scancode),
            InputEventDto::KeyUp { scancode } => InputEvent::KeyUp(release_scancode(scancode)),
            InputEventDto::MousePosition { x, y, buttons } => InputEvent::MousePosition {
                x, y, buttons, display: 0,
            },
            InputEventDto::MouseMotion { dx, dy, buttons } => InputEvent::MouseMotion { dx, dy, buttons },
            InputEventDto::MousePress { button, buttons } => InputEvent::MousePress { button, buttons },
            InputEventDto::MouseRelease { button, buttons } => InputEvent::MouseRelease { button, buttons },
        }
    }
}

// ── Commands ───────────────────────────────────────────────────────────

/// Open a SPICE session for a VM and begin forwarding events.
/// Returns the surface dimensions the moment a SurfaceCreated arrives (or 0x0 on timeout).
#[tauri::command]
pub fn open_spice(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
    #[allow(non_snake_case)]
    password: Option<String>,
) -> Result<(), VirtManagerError> {
    state.close_spice();

    // Parse VM XML for the SPICE endpoint. We deliberately do NOT use
    // VIR_DOMAIN_XML_SECURE to fetch the ticket: auto-extracting a
    // server-configured password defeats the purpose of ticket auth.
    // The password must be provided by the user.
    let xml = state.libvirt().get_domain_xml(&name, false)?;
    let (listen, port) = spice_proxy::parse_spice_endpoint(&xml).ok_or_else(|| {
        VirtManagerError::OperationFailed {
            operation: "parseSpiceEndpoint".into(),
            reason: "VM has no active SPICE graphics port".into(),
        }
    })?;
    let password = password.unwrap_or_default();

    let uri = state.current_uri().ok_or(VirtManagerError::NotConnected)?;
    let ssh_target = vnc_proxy::parse_ssh_target(&uri).ok_or_else(|| {
        VirtManagerError::OperationFailed {
            operation: "parseUri".into(),
            reason: format!("SPICE requires qemu+ssh:// URI; got: {uri}"),
        }
    })?;

    let mut session = SpiceSession::start(
        &ssh_target,
        &listen,
        port,
        &password,
        state.runtime_handle(),
    )
    .map_err(|e| {
        let s = e.to_string().to_lowercase();
        if s.contains("permission") || s.contains("auth") || s.contains("ticket") {
            VirtManagerError::SpiceAuthRequired
        } else {
            e
        }
    })?;

    // Take the event receiver out so we can spawn a task that forwards to Tauri.
    let (fake_tx, events_rx) = tokio::sync::mpsc::channel(1);
    let mut real_rx = std::mem::replace(&mut session.events_rx, events_rx);
    drop(fake_tx);

    let app_handle = app.clone();
    state.runtime_handle().spawn(async move {
        while let Some(evt) = real_rx.recv().await {
            if let Some(dto) = to_dto(evt) {
                if app_handle.emit("spice:event", dto).is_err() {
                    break;
                }
            }
        }
    });

    state.set_spice(session);
    Ok(())
}

#[tauri::command]
pub fn close_spice(state: State<'_, AppState>) {
    state.close_spice();
}

#[tauri::command]
pub async fn spice_input(
    state: State<'_, AppState>,
    event: InputEventDto,
) -> Result<(), VirtManagerError> {
    // Snapshot the sender (cheap clone) then drop the mutex BEFORE awaiting,
    // so we can use async send().await instead of try_send. Dropping a
    // KeyUp because the queue was full produces a sticky-key bug; prefer
    // backpressure over drops for keyboard input.
    let sender = state.spice_sender().ok_or_else(|| VirtManagerError::OperationFailed {
        operation: "spiceInput".into(),
        reason: "no active SPICE session".into(),
    })?;
    sender.send(event.into()).await.map_err(|_| VirtManagerError::OperationFailed {
        operation: "spiceInput".into(),
        reason: "SPICE session closed".into(),
    })
}


#[cfg(test)]
mod tests {
    use super::release_scancode;

    #[test]
    fn release_sets_break_bit_on_short_scancode() {
        // 'A' key (0x1E) -> 0x9E
        assert_eq!(release_scancode(0x1E), 0x9E);
    }

    #[test]
    fn release_preserves_extended_high_byte() {
        // Right arrow (0xE04D) -> 0xE0CD (high byte 0xE0 stays; low byte 0x4D | 0x80)
        assert_eq!(release_scancode(0xE04D), 0xE0CD);
    }

    #[test]
    fn release_idempotent_on_already_released() {
        // If someone double-releases, bit 0x80 is already set — stays set.
        assert_eq!(release_scancode(0x9E), 0x9E);
    }

    #[test]
    fn release_does_not_touch_unrelated_high_bits() {
        // Hypothetical multi-byte scancode: make sure we only twiddle
        // the low byte, not any pattern that happens to include 0x80
        // in a higher byte.
        assert_eq!(release_scancode(0x1E80_1E), 0x1E80_9E);
    }
}
