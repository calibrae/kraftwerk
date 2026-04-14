pub mod connection;
pub mod console;
pub mod domain_config;
pub mod domain_stats;
pub mod hostdev;
pub mod domain_caps;
pub mod boot_config;
pub mod virtio_devices;
pub mod domain_builder;
pub mod network_config;
pub mod storage_config;
pub mod vnc_proxy;
pub mod spice_proxy;
pub mod xml_helpers;

#[cfg(test)]
pub mod test_helpers;
