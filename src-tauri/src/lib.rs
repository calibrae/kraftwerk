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
use commands::nics as cmd_nics;
use commands::display as cmd_display;
use commands::virtio as cmd_virtio;
use commands::char_devices as cmd_char_devices;
use commands::filesystem as cmd_filesystem;
use commands::controllers as cmd_controllers;
use commands::cpu_tune as cmd_cpu_tune;
use commands::storage;
use commands::host;
use commands::snapshots as snap;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let config_path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("kraftwerk")
        .join("connections.json");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::with_persistence(config_path))
        .setup(|app| {
            use tauri::Manager; use tauri::Emitter;
            let state: tauri::State<'_, AppState> = app.state();
            if let Some(mut rx) = state.take_event_rx() {
                let handle = app.handle().clone();
                state.runtime().spawn(async move {
                    while let Some(ev) = rx.recv().await {
                        let _ = handle.emit("domain_event", &ev);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Connection management
            connection::add_connection,
            connection::update_connection,
            connection::remove_connection,
            connection::list_saved_connections,
            connection::check_host_key,
            connection::accept_host_key,
            connection::forget_host_key,
            host::get_host_info,
            host::get_host_memory,
            snap::list_snapshots,
            snap::create_snapshot,
            snap::revert_snapshot,
            snap::delete_snapshot,
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
            domain::set_max_memory_mb,
            domain::define_domain,
            domain::clone_domain,
            domain::get_qemu_log,
            domain::managed_save_domain,
            domain::has_managed_save,
            domain::managed_save_remove,
            domain::core_dump_domain,
            domain::screenshot_domain,
            domain::get_backing_chains,
            domain::block_pull,
            domain::block_commit,
            domain::get_block_job,
            domain::block_job_abort,
            domain::get_memory_hotplug,
            domain::set_max_memory_slots,
            domain::attach_memory_dimm,
            domain::set_max_vcpus_count,
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
            network::add_dhcp_host,
            network::remove_dhcp_host,
            network::add_dns_host,
            network::remove_dns_host,
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
            storage::upload_volume,
            storage::list_secrets,
            storage::define_secret,
            storage::set_secret_value,
            storage::delete_secret,
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
            cmd_nics::list_domain_nics,
            cmd_nics::add_domain_nic,
            cmd_nics::remove_domain_nic,
            cmd_nics::update_domain_nic,
            cmd_display::get_display_config,
            cmd_display::apply_display_patch,
            cmd_virtio::get_virtio_devices,
            cmd_virtio::set_tpm,
            cmd_virtio::set_watchdog,
            cmd_virtio::set_panic,
            cmd_virtio::set_balloon,
            cmd_virtio::set_vsock,
            cmd_virtio::add_rng,
            cmd_virtio::remove_rng,
            cmd_virtio::update_rng,
            cmd_virtio::set_iommu,
            // Char devices (Round F)
            cmd_char_devices::get_char_devices,
            cmd_char_devices::add_channel,
            cmd_char_devices::remove_channel,
            cmd_char_devices::add_serial,
            cmd_char_devices::remove_serial,
            cmd_char_devices::add_guest_agent_channel,
            cmd_char_devices::add_spice_vdagent_channel,
            cmd_filesystem::list_filesystems,
            cmd_filesystem::add_filesystem,
            cmd_filesystem::remove_filesystem,
            cmd_filesystem::update_filesystem,
            cmd_filesystem::list_shmems,
            cmd_filesystem::add_shmem,
            cmd_filesystem::remove_shmem,
            cmd_filesystem::enable_shared_memory_backing,
            cmd_controllers::list_controllers,
            cmd_controllers::add_controller,
            cmd_controllers::remove_controller,
            cmd_controllers::update_controller,
            cmd_cpu_tune::get_cpu_tune,
            cmd_cpu_tune::apply_cpu_tune,
            cmd_cpu_tune::set_vcpu_count,
            cmd_cpu_tune::set_iothread_count,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
