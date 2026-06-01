//! Tauri commands for the in-process log ring buffer.

use tauri::State;

use crate::app_state::AppState;
use crate::log_buffer::{level_filter_from_str, LogEntry};
use crate::models::error::VirtManagerError;

/// Fetch buffered log entries. `after_ts_ms` (optional, exclusive) lets
/// the UI tail incrementally — pass back the largest ts_ms from the
/// previous batch. `min_level` filters by minimum severity ("error" |
/// "warn" | "info" | "debug" | "trace"); `None` = no filter.
#[tauri::command]
pub fn get_logs(
    state: State<'_, AppState>,
    after_ts_ms: Option<u64>,
    min_level: Option<String>,
) -> Result<Vec<LogEntry>, VirtManagerError> {
    let Some(buf) = state.log_buffer() else {
        return Ok(Vec::new());
    };
    let lvl = min_level.as_deref().and_then(|s| match s.to_ascii_lowercase().as_str() {
        "error" => Some(log::Level::Error),
        "warn" => Some(log::Level::Warn),
        "info" => Some(log::Level::Info),
        "debug" => Some(log::Level::Debug),
        "trace" => Some(log::Level::Trace),
        _ => None,
    });
    Ok(buf.filtered(after_ts_ms, lvl))
}

/// Drop every entry currently in the buffer.
#[tauri::command]
pub fn clear_logs(state: State<'_, AppState>) -> Result<(), VirtManagerError> {
    if let Some(buf) = state.log_buffer() {
        buf.clear();
    }
    Ok(())
}

/// Change the active log level at runtime. Accepts "off" | "error" |
/// "warn" | "info" | "debug" | "trace". The UI "verbose" toggle maps
/// to "debug" on / "info" off.
#[tauri::command]
pub fn set_log_level(level: String) -> Result<(), VirtManagerError> {
    let f = level_filter_from_str(&level).ok_or_else(|| VirtManagerError::OperationFailed {
        operation: "setLogLevel".into(),
        reason: format!("unknown level '{level}'"),
    })?;
    log::set_max_level(f);
    log::info!("log level set to {f}");
    Ok(())
}

/// Read the currently-active level filter as a lowercase string.
#[tauri::command]
pub fn get_log_level() -> String {
    log::max_level().to_string().to_lowercase()
}
