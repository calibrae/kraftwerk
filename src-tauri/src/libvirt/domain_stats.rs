//! Snapshot collector for live VM metrics.
//!
//! Each `sample()` call pulls current counters from libvirt. The frontend
//! computes deltas between two samples to get rates (CPU %, bytes/sec).

use serde::{Deserialize, Serialize};
use virt::connect::Connect;
use virt::domain::Domain;

use crate::libvirt::xml_helpers;
use crate::models::error::VirtManagerError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainStatsSample {
    /// Milliseconds since UNIX epoch for this sample.
    pub timestamp_ms: u64,
    /// Cumulative CPU time in nanoseconds across all vCPUs.
    pub cpu_time_ns: u64,
    pub vcpus: u32,
    /// Current memory the balloon driver reports in use (KiB).
    /// If the balloon driver isn't running, this is 0 and `memory_actual_kib` is used.
    pub memory_rss_kib: u64,
    /// Memory the hypervisor has allocated to the domain (KiB).
    pub memory_actual_kib: u64,
    /// Max memory configured for the domain (KiB).
    pub memory_max_kib: u64,
    /// Per-disk counters.
    pub disks: Vec<DiskSample>,
    /// Per-NIC counters.
    pub interfaces: Vec<InterfaceSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskSample {
    pub device: String, // "vda", "sda", etc.
    pub read_bytes: i64,
    pub write_bytes: i64,
    pub read_req: i64,
    pub write_req: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceSample {
    /// Guest-side device name (e.g. "vnet3") — what libvirt expects for lookup.
    pub path: String,
    /// Model + MAC for display purposes.
    pub mac: String,
    pub model: String,
    pub rx_bytes: i64,
    pub tx_bytes: i64,
    pub rx_packets: i64,
    pub tx_packets: i64,
}

/// Collect a single snapshot of a domain's stats.
pub fn sample(conn: &Connect, name: &str) -> Result<DomainStatsSample, VirtManagerError> {
    let domain = Domain::lookup_by_name(conn, name).map_err(|_| {
        VirtManagerError::DomainNotFound { name: name.to_string() }
    })?;

    let info = domain.get_info().map_err(|e| VirtManagerError::OperationFailed {
        operation: "getDomainInfo".into(),
        reason: e.to_string(),
    })?;

    let xml = domain.get_xml_desc(0).map_err(|e| VirtManagerError::OperationFailed {
        operation: "getDomainXML".into(),
        reason: e.to_string(),
    })?;

    // Disk & NIC targets from the XML
    let disk_devs = xml_helpers::extract_disk_targets(&xml);
    let iface_paths = xml_helpers::extract_interface_targets(&xml);

    let mut disks = Vec::with_capacity(disk_devs.len());
    for dev in &disk_devs {
        if let Ok(bs) = domain.get_block_stats(dev) {
            disks.push(DiskSample {
                device: dev.clone(),
                read_bytes: bs.rd_bytes,
                write_bytes: bs.wr_bytes,
                read_req: bs.rd_req,
                write_req: bs.wr_req,
            });
        }
    }

    let mut interfaces = Vec::with_capacity(iface_paths.len());
    for nic in &iface_paths {
        if let Ok(s) = domain.interface_stats(&nic.path) {
            interfaces.push(InterfaceSample {
                path: nic.path.clone(),
                mac: nic.mac.clone(),
                model: nic.model.clone(),
                rx_bytes: s.rx_bytes,
                tx_bytes: s.tx_bytes,
                rx_packets: s.rx_packets,
                tx_packets: s.tx_packets,
            });
        }
    }

    // Memory: try balloon stats for "rss" (guest's reported used memory)
    let mut memory_rss_kib: u64 = 0;
    if let Ok(stats) = domain.memory_stats(0) {
        for stat in stats {
            // tag 7 = VIR_DOMAIN_MEMORY_STAT_RSS, tag 6 = AVAILABLE
            if stat.tag == 7 {
                memory_rss_kib = stat.val;
                break;
            }
        }
    }

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    Ok(DomainStatsSample {
        timestamp_ms,
        cpu_time_ns: info.cpu_time,
        vcpus: info.nr_virt_cpu,
        memory_rss_kib,
        memory_actual_kib: info.memory,
        memory_max_kib: info.max_mem,
        disks,
        interfaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_sample() {
        let s = DomainStatsSample {
            timestamp_ms: 1,
            cpu_time_ns: 1_000_000_000,
            vcpus: 2,
            memory_rss_kib: 0,
            memory_actual_kib: 2048 * 1024,
            memory_max_kib: 2048 * 1024,
            disks: vec![DiskSample {
                device: "vda".into(),
                read_bytes: 100, write_bytes: 200, read_req: 10, write_req: 20,
            }],
            interfaces: vec![InterfaceSample {
                path: "vnet0".into(), mac: "52:54:00:aa:bb:cc".into(), model: "virtio".into(),
                rx_bytes: 1_000_000, tx_bytes: 500_000, rx_packets: 1000, tx_packets: 500,
            }],
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"cpu_time_ns\":1000000000"));
        assert!(json.contains("\"device\":\"vda\""));
        assert!(json.contains("\"mac\":\"52:54:00:aa:bb:cc\""));
    }
}
