use tauri::{Emitter, State};

use crate::app_state::AppState;
use crate::models::error::VirtManagerError;

/// Open a serial console session for a VM.
/// Data from the VM is emitted as `console:data` events with base64-encoded bytes.
#[tauri::command]
pub fn open_console(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), VirtManagerError> {
    // Close existing session first
    state.close_console();

    let session = state.open_console(&name, move |data| {
        // Emit raw bytes as a Vec<u8> — Tauri serializes as JSON array of numbers
        let _ = app.emit("console:data", data);
    })?;

    state.set_console(session);
    Ok(())
}

/// Send input bytes to the active console session.
#[tauri::command]
pub fn console_send(
    state: State<'_, AppState>,
    data: Vec<u8>,
) -> Result<(), VirtManagerError> {
    state.console_send(&data)
}

/// Close the active console session.
#[tauri::command]
pub fn close_console(state: State<'_, AppState>) -> Result<(), VirtManagerError> {
    state.close_console();
    Ok(())
}

/// Check if a console session is active.
#[tauri::command]
pub fn console_is_active(state: State<'_, AppState>) -> bool {
    state.console_is_active()
}
