//! Host (hypervisor node) information for the dashboard view shown when
//! a connection is selected but no VM is highlighted.
//!
//! Pulled from `virConnectGetHostname`, `virNodeGetInfo`, `virConnectGetType`,
//! `virConnectGetLibVersion`, and `virNodeGetFreeMemory`. All cheap calls,
//! no XML parsing needed.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
    pub hostname: String,
    pub hypervisor_type: String,
    pub libvirt_version: String,
    pub cpu_model: String,
    pub cpu_count: u32,
    pub cpu_mhz: u32,
    pub cpu_sockets: u32,
    pub cpu_cores_per_socket: u32,
    pub cpu_threads_per_core: u32,
    pub numa_nodes: u32,
    /// Total RAM in KiB.
    pub memory_kib: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostMemory {
    pub total_kib: u64,
    pub free_kib: u64,
}

/// Format a libvirt-style version u32 (1000000 * major + 1000 * minor + release)
/// into a human-readable "M.m.r" string.
pub fn format_lib_version(v: u32) -> String {
    let major = v / 1_000_000;
    let minor = (v / 1_000) % 1_000;
    let release = v % 1_000;
    format!("{major}.{minor}.{release}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_lib_version() {
        assert_eq!(format_lib_version(9_000_000), "9.0.0");
        assert_eq!(format_lib_version(9_010_002), "9.10.2");
        assert_eq!(format_lib_version(11_005_000), "11.5.0");
    }
}
