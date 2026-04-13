pub mod app_state;
mod commands;
pub mod libvirt;
pub mod models;

use app_state::AppState;
use commands::connection;
use commands::console;
use commands::domain;
use commands::network;

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
            domain::get_domain_config,
            domain::set_vcpus,
            domain::set_memory_mb,
            // Console
            console::open_console,
            console::console_send,
            console::close_console,
            console::console_is_active,
            // Networks
            network::list_networks,
            network::get_network_config,
            network::get_network_xml,
            network::start_network,
            network::stop_network,
            network::delete_network,
            network::set_network_autostart,
            network::create_nat_network,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
