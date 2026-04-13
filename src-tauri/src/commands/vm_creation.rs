use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::domain_builder::{self, DiskSource, DomainBuildParams};
use crate::libvirt::storage_config::{build_volume_xml, VolumeBuildParams};
use crate::models::error::VirtManagerError;
use crate::models::os_variants::{self, OsVariant};

/// Return the list of known OS variants for the wizard.
#[tauri::command]
pub fn list_os_variants() -> Vec<OsVariant> {
    os_variants::all_variants()
}

/// Create a new VM from structured parameters.
///
/// Handles:
/// - If disk_source is NewVolume: creates the volume in the pool first,
///   then swaps the disk_source to ExistingPath before building XML.
/// - Builds domain XML and defines the domain.
/// - Optionally starts the VM.
#[tauri::command]
pub fn create_vm(
    state: State<'_, AppState>,
    mut params: DomainBuildParams,
    start: bool,
) -> Result<String, VirtManagerError> {
    let lv = state.libvirt();

    // If we need to create a new volume, do that first.
    if let DiskSource::NewVolume {
        pool_name,
        name,
        capacity_bytes,
        format,
    } = params.disk_source.clone()
    {
        let vol_xml = build_volume_xml(&VolumeBuildParams {
            name: &name,
            capacity_bytes,
            format: &format,
            allocation_bytes: None,
        });
        let path = lv.create_volume(&pool_name, &vol_xml)?;
        params.disk_source = DiskSource::ExistingPath { path, format };
    }

    let xml = domain_builder::build_domain_xml(&params);
    lv.define_domain_xml(&xml)?;

    if start {
        lv.start_domain(&params.name)?;
    }
    Ok(xml)
}
