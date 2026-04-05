pub mod app_state;
mod commands;
pub mod libvirt;
pub mod models;

use app_state::AppState;
use commands::connection;
use commands::domain;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            // Connection management
            connection::add_connection,
            connection::remove_connection,
            connection::list_saved_connections,
            connection::connect,
            connection::disconnect,
            connection::get_connection_state,
            // Domain operations
            domain::list_domains,
            domain::start_domain,
            domain::shutdown_domain,
            domain::destroy_domain,
            domain::suspend_domain,
            domain::resume_domain,
            domain::reboot_domain,
            domain::get_domain_xml,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
