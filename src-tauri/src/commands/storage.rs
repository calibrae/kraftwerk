use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::app_state::AppState;
use crate::libvirt::storage_config::{self, PoolBuildParams, VolumeBuildParams};
use crate::models::error::VirtManagerError;
use crate::models::storage::{StoragePoolInfo, StorageVolumeInfo};

#[derive(Debug, Deserialize)]
pub struct CreatePoolRequest {
    pub name: String,
    /// "dir" | "netfs" | "logical" | "iscsi" | "iscsi-direct" | "rbd"
    pub pool_type: String,
    pub target_path: Option<String>,
    pub source_host: Option<String>,
    pub source_dir: Option<String>,
    pub source_name: Option<String>,
    /// Optional auth for iSCSI (CHAP) and RBD (Ceph).
    pub auth: Option<PoolAuthRequest>,
    #[serde(default = "default_true")]
    pub build: bool,
    #[serde(default = "default_true")]
    pub start: bool,
    #[serde(default)]
    pub autostart: bool,
}

#[derive(Debug, Deserialize)]
pub struct PoolAuthRequest {
    /// "chap" for iSCSI, "ceph" for RBD.
    pub auth_type: String,
    pub username: String,
    pub secret_uuid: String,
    pub secret_usage: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVolumeRequest {
    pub pool_name: String,
    pub name: String,
    pub capacity_bytes: u64,
    /// "qcow2" | "raw" | "iso"
    pub format: String,
    pub allocation_bytes: Option<u64>,
    /// When set, the volume is created as a LUKS container referencing
    /// this existing secret UUID. The secret must already have its
    /// passphrase set via set_secret_value before create_volume runs;
    /// libvirt uses it to initialise the LUKS header.
    #[serde(default)]
    pub luks_secret_uuid: Option<String>,
}

fn default_true() -> bool { true }

// ── Pool commands ──

#[tauri::command]
pub fn list_storage_pools(state: State<'_, AppState>) -> Result<Vec<StoragePoolInfo>, VirtManagerError> {
    state.libvirt().list_storage_pools()
}

#[tauri::command]
pub fn get_pool_xml(state: State<'_, AppState>, name: String) -> Result<String, VirtManagerError> {
    state.libvirt().get_pool_xml(&name)
}

#[tauri::command]
pub fn get_pool_config(
    state: State<'_, AppState>,
    name: String,
) -> Result<storage_config::PoolConfig, VirtManagerError> {
    state.libvirt().get_pool_config(&name)
}

#[tauri::command]
pub fn start_pool(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().start_pool(&name)
}

#[tauri::command]
pub fn stop_pool(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().stop_pool(&name)
}

#[tauri::command]
pub fn refresh_pool(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().refresh_pool(&name)
}

#[tauri::command]
pub fn delete_pool(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    // Guard: refuse if the pool still has volumes. libvirt rejects this
    // with an opaque error; we wrap it with a clearer one.
    if let Ok(vols) = state.libvirt().list_volumes(&name) {
        if !vols.is_empty() {
            return Err(VirtManagerError::OperationFailed {
                operation: "deletePool".into(),
                reason: format!(
                    "Pool {} has {} volumes; delete them first.",
                    name,
                    vols.len()
                ),
            });
        }
    }
    state.libvirt().delete_pool(&name)
}

#[tauri::command]
pub fn set_pool_autostart(
    state: State<'_, AppState>,
    name: String,
    autostart: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_pool_autostart(&name, autostart)
}

#[tauri::command]
pub fn create_pool(
    state: State<'_, AppState>,
    req: CreatePoolRequest,
) -> Result<(), VirtManagerError> {
    let auth = req.auth.as_ref().map(|a| crate::libvirt::storage_config::PoolAuthParams {
        auth_type: &a.auth_type,
        username: &a.username,
        secret_uuid: &a.secret_uuid,
        secret_usage: a.secret_usage.as_deref(),
    });
    let xml = storage_config::build_pool_xml(&PoolBuildParams {
        name: &req.name,
        pool_type: &req.pool_type,
        target_path: req.target_path.as_deref(),
        source_host: req.source_host.as_deref(),
        source_dir: req.source_dir.as_deref(),
        source_name: req.source_name.as_deref(),
        auth,
    });
    state.libvirt().define_pool(&xml, req.build, req.start)?;
    if req.autostart {
        let _ = state.libvirt().set_pool_autostart(&req.name, true);
    }
    Ok(())
}

// ── Volume commands ──

#[tauri::command]
pub fn list_volumes(
    state: State<'_, AppState>,
    pool_name: String,
) -> Result<Vec<StorageVolumeInfo>, VirtManagerError> {
    state.libvirt().list_volumes(&pool_name)
}

#[tauri::command]
pub fn create_volume(
    state: State<'_, AppState>,
    req: CreateVolumeRequest,
) -> Result<String, VirtManagerError> {
    let xml = if let Some(uuid) = req.luks_secret_uuid.as_deref().filter(|s| !s.is_empty()) {
        // LUKS bypasses the regular builder — capacity+name+secret-ref only.
        crate::libvirt::secrets::build_luks_volume_xml(&req.name, req.capacity_bytes, uuid)
    } else {
        storage_config::build_volume_xml(&VolumeBuildParams {
            name: &req.name,
            capacity_bytes: req.capacity_bytes,
            format: &req.format,
            allocation_bytes: req.allocation_bytes,
        })
    };
    state.libvirt().create_volume(&req.pool_name, &xml)
}

// ── Secret (libvirt-managed credential) commands ──

#[derive(Debug, serde::Deserialize)]
pub struct DefineSecretRequest {
    /// "none" | "volume" | "ceph" | "iscsi" | "tls" | "vtpm"
    pub usage: String,
    pub usage_id: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default = "default_true")]
    pub private: bool,
    /// Optional passphrase / token. If provided, set_secret_value is
    /// called immediately after define so the caller doesn't have to
    /// round-trip.
    pub value: Option<String>,
}

#[tauri::command]
pub fn list_secrets(
    state: State<'_, AppState>,
) -> Result<Vec<crate::libvirt::secrets::SecretInfo>, VirtManagerError> {
    state.libvirt().list_secrets()
}

#[tauri::command]
pub fn define_secret(
    state: State<'_, AppState>,
    req: DefineSecretRequest,
) -> Result<String, VirtManagerError> {
    let usage = match req.usage.as_str() {
        "none" => crate::libvirt::secrets::SecretUsage::None,
        "volume" => crate::libvirt::secrets::SecretUsage::Volume,
        "ceph" => crate::libvirt::secrets::SecretUsage::Ceph,
        "iscsi" => crate::libvirt::secrets::SecretUsage::Iscsi,
        "tls" => crate::libvirt::secrets::SecretUsage::Tls,
        "vtpm" => crate::libvirt::secrets::SecretUsage::Vtpm,
        _ => return Err(VirtManagerError::OperationFailed {
            operation: "defineSecret".into(),
            reason: format!("unsupported usage type {:?}", req.usage),
        }),
    };
    let uuid = state.libvirt().define_secret(
        usage,
        req.usage_id.as_deref(),
        req.description.as_deref(),
        req.ephemeral,
        req.private,
    )?;
    if let Some(v) = req.value.as_deref() {
        state.libvirt().set_secret_value(&uuid, v.as_bytes())?;
    }
    Ok(uuid)
}

#[tauri::command]
pub fn set_secret_value(
    state: State<'_, AppState>,
    uuid: String,
    value: String,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_secret_value(&uuid, value.as_bytes())
}

#[tauri::command]
pub fn delete_secret(
    state: State<'_, AppState>,
    uuid: String,
) -> Result<(), VirtManagerError> {
    state.libvirt().delete_secret(&uuid)
}

#[tauri::command]
pub fn delete_volume(state: State<'_, AppState>, path: String) -> Result<(), VirtManagerError> {
    // Guard: refuse if any domain currently references this volume path.
    let domains = state.libvirt().list_all_domains()?;
    let mut in_use_by: Vec<String> = Vec::new();
    let needle_file_sq = format!("file='{}'", path);
    let needle_file_dq = format!("file=\"{}\"", path);
    let needle_dev_sq = format!("dev='{}'", path);
    let needle_dev_dq = format!("dev=\"{}\"", path);
    for d in &domains {
        let xml = match state.libvirt().get_domain_xml(&d.name, true) {
            Ok(x) => x,
            Err(_) => continue,
        };
        if xml.contains(&needle_file_sq) || xml.contains(&needle_file_dq)
            || xml.contains(&needle_dev_sq) || xml.contains(&needle_dev_dq)
        {
            in_use_by.push(d.name.clone());
        }
    }
    if !in_use_by.is_empty() {
        return Err(VirtManagerError::OperationFailed {
            operation: "deleteVolume".into(),
            reason: format!(
                "Volume {} is in use by domain(s): {}. Detach it from those VMs first.",
                path,
                in_use_by.join(", ")
            ),
        });
    }
    state.libvirt().delete_volume(&path)
}

#[tauri::command]
pub fn resize_volume(
    state: State<'_, AppState>,
    path: String,
    capacity_bytes: u64,
) -> Result<(), VirtManagerError> {
    state.libvirt().resize_volume(&path, capacity_bytes)
}

#[derive(Debug, Clone, Serialize)]
pub struct VolumeUploadProgress {
    pub pool_name: String,
    pub vol_name: String,
    pub sent: u64,
    pub total: u64,
}

/// Stream a local file's contents into an existing volume. The volume
/// is identified by (pool_name, vol_name) — call create_volume first.
/// Progress events fire on the `volume_upload_progress` Tauri channel
/// throttled to ~5/sec so the webview doesn't drown.
#[tauri::command]
pub fn upload_volume(
    state: State<'_, AppState>,
    app: AppHandle,
    pool_name: String,
    vol_name: String,
    source_path: String,
) -> Result<u64, VirtManagerError> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    let last = Mutex::new(Instant::now() - Duration::from_secs(1));
    let pool_for_cb = pool_name.clone();
    let vol_for_cb = vol_name.clone();
    state.libvirt().upload_volume_from_path(
        &pool_name,
        &vol_name,
        &source_path,
        1024 * 1024,
        |sent, total| {
            let mut l = match last.lock() { Ok(g) => g, Err(p) => p.into_inner() };
            let now = Instant::now();
            if now.duration_since(*l) >= Duration::from_millis(200) || sent == 0 || sent == total {
                *l = now;
                let _ = app.emit("volume_upload_progress", VolumeUploadProgress {
                    pool_name: pool_for_cb.clone(),
                    vol_name: vol_for_cb.clone(),
                    sent,
                    total,
                });
            }
        },
    )
}
