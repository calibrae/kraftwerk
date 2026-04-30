//! Tauri commands for the cloud image catalog.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::image_catalog::{builtin_catalog, CatalogImage, CatalogImageStatus};
use crate::models::error::VirtManagerError;

/// The static catalog. Doesn't need a connection.
#[tauri::command]
pub fn list_image_catalog() -> Vec<CatalogImage> {
    builtin_catalog()
}

/// Catalog joined against a specific pool's existing volumes — tells
/// the UI which images are already on disk for that pool.
#[tauri::command]
pub fn list_image_catalog_for_pool(
    state: State<'_, AppState>,
    pool_name: String,
) -> Result<Vec<CatalogImageStatus>, VirtManagerError> {
    state.libvirt().list_catalog_images(&pool_name)
}

/// Download a catalog image into the named pool. Streams the bytes
/// over SSH+curl on the hypervisor host so the file lands directly
/// in the pool's target dir, then refreshes the pool.
#[tauri::command]
pub fn download_image(
    state: State<'_, AppState>,
    image_id: String,
    pool_name: String,
) -> Result<String, VirtManagerError> {
    state.libvirt().download_catalog_image(&image_id, &pool_name)
}
