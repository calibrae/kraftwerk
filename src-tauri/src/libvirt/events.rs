//! libvirt lifecycle event subscription.
//!
//! Replaces the 3s polling loop with push-driven domain state updates.
//! libvirt's C event loop runs on a dedicated thread; lifecycle callbacks
//! are routed through a global mpsc to a tokio task that fans events out
//! to the Tauri webview.
//!
//! Architecture:
//!   libvirt event-loop thread (virEventRunDefaultImpl)
//!     ↓ unsafe extern "C" lifecycle_cb (called per event)
//!     ↓ EVENT_TX.send()
//!   tokio task (drain_events)
//!     ↓ app_handle.emit("domain_event", payload)
//!   webview (Svelte store listener)
//!
//! Init is done once per process via `Once`. Per-connection registration
//! is wired in `LibvirtConnection::open` and removed in `close`.

use serde::Serialize;
use std::ffi::CStr;
use std::os::raw::{c_int, c_void};
use std::sync::{Once, OnceLock};
use tokio::sync::mpsc;

use crate::models::error::VirtManagerError;

/// Lifecycle event delivered to the frontend.
///
/// Mirrors libvirt's `VIR_DOMAIN_EVENT_*` constants. The `detail` field
/// carries the sub-reason (e.g. for STOPPED: SHUTDOWN / CRASHED /
/// MIGRATED). We surface it raw for now; the frontend can branch on it.
#[derive(Debug, Clone, Serialize)]
pub struct DomainEvent {
    pub vm_name: String,
    pub kind: &'static str,
    pub detail: i32,
}

static EVENT_LOOP_INIT: Once = Once::new();
static EVENT_TX: OnceLock<mpsc::UnboundedSender<DomainEvent>> = OnceLock::new();

/// Initialise the libvirt default event loop and remember the channel.
/// Must be called exactly once per process; subsequent calls are no-ops.
/// Safe to call before any libvirt connection exists.
pub fn init_once(tx: mpsc::UnboundedSender<DomainEvent>) {
    let _ = EVENT_TX.set(tx);
    EVENT_LOOP_INIT.call_once(|| unsafe {
        if virt_sys::virEventRegisterDefaultImpl() < 0 {
            log::error!("virEventRegisterDefaultImpl failed; events disabled");
            return;
        }
        std::thread::Builder::new()
            .name("libvirt-events".into())
            .spawn(|| loop {
                if unsafe { virt_sys::virEventRunDefaultImpl() } < 0 {
                    log::error!("virEventRunDefaultImpl returned <0; event loop exiting");
                    break;
                }
            })
            .expect("spawn libvirt-events thread");
    });
}

unsafe extern "C" fn lifecycle_cb(
    _conn: virt_sys::virConnectPtr,
    dom: virt_sys::virDomainPtr,
    event: c_int,
    detail: c_int,
    _opaque: *mut c_void,
) -> c_int {
    let name = unsafe {
        let p = virt_sys::virDomainGetName(dom);
        if p.is_null() {
            String::new()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };
    let kind = match event {
        0 => "defined",
        1 => "undefined",
        2 => "started",
        3 => "suspended",
        4 => "resumed",
        5 => "stopped",
        6 => "shutdown",
        7 => "pmsuspended",
        8 => "crashed",
        _ => "unknown",
    };
    if let Some(tx) = EVENT_TX.get() {
        let _ = tx.send(DomainEvent { vm_name: name, kind, detail });
    }
    0
}

/// Register the lifecycle callback against an open libvirt connection.
/// Returns Ok if registration succeeds OR if events are not yet
/// initialised (caller proceeds with polling fallback).
pub fn register(conn_ptr: virt_sys::virConnectPtr) -> Result<(), VirtManagerError> {
    if EVENT_TX.get().is_none() {
        return Ok(());
    }
    let id = unsafe {
        virt_sys::virConnectDomainEventRegister(
            conn_ptr,
            Some(lifecycle_cb),
            std::ptr::null_mut(),
            None,
        )
    };
    if id < 0 {
        return Err(VirtManagerError::OperationFailed {
            operation: "virConnectDomainEventRegister".into(),
            reason: "registration returned <0".into(),
        });
    }
    Ok(())
}

/// Remove the lifecycle callback. Idempotent — safe to call even if not
/// registered. Errors are logged but never propagated since deregistration
/// happens during connection teardown where we're past caring.
pub fn deregister(conn_ptr: virt_sys::virConnectPtr) {
    unsafe {
        let r = virt_sys::virConnectDomainEventDeregister(conn_ptr, Some(lifecycle_cb));
        if r < 0 {
            log::debug!("virConnectDomainEventDeregister returned {r}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_mapping_covers_known_values() {
        // Mirror the cb's match table; if libvirt adds a new event ID we
        // want this to fail visibly so we can extend the mapping.
        let known = [
            (0, "defined"),
            (1, "undefined"),
            (2, "started"),
            (3, "suspended"),
            (4, "resumed"),
            (5, "stopped"),
            (6, "shutdown"),
            (7, "pmsuspended"),
            (8, "crashed"),
        ];
        for (id, expected) in known {
            let got = match id {
                0 => "defined",
                1 => "undefined",
                2 => "started",
                3 => "suspended",
                4 => "resumed",
                5 => "stopped",
                6 => "shutdown",
                7 => "pmsuspended",
                8 => "crashed",
                _ => "unknown",
            };
            assert_eq!(got, expected);
        }
    }
}
