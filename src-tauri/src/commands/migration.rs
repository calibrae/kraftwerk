//! Tauri commands for live migration.

use tauri::State;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::libvirt::migration::{MigrationConfig, MigrationProgress};
use crate::models::error::VirtManagerError;

fn parse_uuid(id: &str) -> Result<Uuid, VirtManagerError> {
    Uuid::parse_str(id).map_err(|_| VirtManagerError::ConnectionNotFound { id: id.into() })
}

/// Migrate a domain from `source_connection_id` to
/// `dest_connection_id`. Both connection ids must reference *open*
/// entries in the connection pool.
///
/// Blocks until libvirt completes the migration; the call may take
/// minutes for large guests. UI should call this from a worker task
/// and poll `get_migration_status` for progress while it runs.
#[tauri::command]
pub fn migrate_domain(
    state: State<'_, AppState>,
    source_connection_id: String,
    dest_connection_id: String,
    name: String,
    config: Option<MigrationConfig>,
) -> Result<(), VirtManagerError> {
    let src_id = parse_uuid(&source_connection_id)?;
    let dst_id = parse_uuid(&dest_connection_id)?;
    if src_id == dst_id {
        return Err(VirtManagerError::OperationFailed {
            operation: "migrate".into(),
            reason: "source and destination connections are the same".into(),
        });
    }
    let src = state.libvirt_for(&src_id).ok_or(VirtManagerError::NotConnected)?;
    let dst = state.libvirt_for(&dst_id).ok_or(VirtManagerError::NotConnected)?;
    let cfg = config.unwrap_or_default();
    src.migrate_to(&name, &dst, &cfg)
}

/// Poll the current migration status for `name` on the source
/// connection. Returns an empty progress (phase=None) when no
/// migration is in flight.
#[tauri::command]
pub fn get_migration_status(
    state: State<'_, AppState>,
    source_connection_id: String,
    name: String,
) -> Result<MigrationProgress, VirtManagerError> {
    let src_id = parse_uuid(&source_connection_id)?;
    let src = state.libvirt_for(&src_id).ok_or(VirtManagerError::NotConnected)?;
    src.migration_status(&name)
}

/// Cancel an in-flight migration for `name`. Maps to virDomainAbortJob.
#[tauri::command]
pub fn cancel_migration(
    state: State<'_, AppState>,
    source_connection_id: String,
    name: String,
) -> Result<(), VirtManagerError> {
    let src_id = parse_uuid(&source_connection_id)?;
    let src = state.libvirt_for(&src_id).ok_or(VirtManagerError::NotConnected)?;
    src.cancel_migration(&name)
}
