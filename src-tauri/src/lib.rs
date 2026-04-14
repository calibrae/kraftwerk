pub mod app_state;
mod commands;
pub mod libvirt;
pub mod models;

use app_state::AppState;
use commands::connection;
use commands::console;
use commands::domain;
use commands::network;
use commands::vm_creation;
use commands::vnc;
use commands::spice;
use commands::hostdev;
use commands::domain_caps as cmd_domain_caps;
use commands::boot as cmd_boot;
use commands::disks as cmd_disks;
use commands::storage;

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
            domain::get_domain_stats,
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
            network::create_network,
            // Storage
            storage::list_storage_pools,
            storage::get_pool_xml,
            storage::get_pool_config,
            storage::start_pool,
            storage::stop_pool,
            storage::refresh_pool,
            storage::delete_pool,
            storage::set_pool_autostart,
            storage::create_pool,
            storage::list_volumes,
            storage::create_volume,
            storage::delete_volume,
            storage::resize_volume,
            // VM creation
            vm_creation::list_os_variants,
            vm_creation::create_vm,
            domain::undefine_domain,
            // VNC
            vnc::open_vnc,
            vnc::close_vnc,
            // SPICE
            spice::open_spice,
            spice::close_spice,
            spice::spice_input,
            // Host device passthrough
            hostdev::list_host_pci_devices,
            hostdev::list_host_usb_devices,
            hostdev::list_domain_hostdevs,
            hostdev::attach_hostdev,
            hostdev::detach_hostdev,
            cmd_domain_caps::get_domain_capabilities,
            cmd_boot::get_boot_config,
            cmd_boot::apply_boot_patch,
            cmd_disks::list_domain_disks,
            cmd_disks::add_domain_disk,
            cmd_disks::remove_domain_disk,
            cmd_disks::update_domain_disk,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
