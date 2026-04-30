//! Live migration of running domains between hypervisors.
//!
//! libvirt's classic 3-step migration: source side prepares, target
//! side claims, source side cleans up. We use the `virDomainMigrate`
//! peer-to-peer flow (libvirt-rs `Domain::migrate`) which delegates
//! the wire-up to libvirt itself — the only thing we manage is the
//! flag bitmask, an optional destination URI override (when the
//! source-side libvirtd can't reach the target on the same address
//! the target expects), and an optional dest XML rewrite.
//!
//! Job progress is observed by polling `virDomainGetJobStats` on the
//! source. Cancellation goes through `virDomainAbortJob` (raw FFI —
//! the safe wrapper doesn't expose it in this crate version).
//!
//! Scope:
//! - Shared-storage migration only. The disk has to be visible to
//!   both source and destination (NFS / iSCSI / Ceph RBD). Storage
//!   migration with `MIGRATE_NON_SHARED_DISK` is a separate feature
//!   and not implemented here.
//! - No tunnelled mode — assumes the operator has direct connectivity
//!   between the QEMU processes on the two hosts.
//! - No TLS — relies on whatever `qemu+ssh` / `qemu+tls` already
//!   provides on the libvirtd transport.

use serde::{Deserialize, Serialize};

use crate::models::error::VirtManagerError;

/// Migration knobs surfaced to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    /// Pre-copy memory while the guest keeps running. Always on for
    /// live migration; setting false here triggers an offline (paused-
    /// during-copy) migration which has worse downtime.
    pub live: bool,
    /// VIR_MIGRATE_PERSIST_DEST — define the domain on the destination
    /// so it survives a libvirtd restart. Without this the domain only
    /// exists transiently on the destination.
    pub persist_dest: bool,
    /// VIR_MIGRATE_UNDEFINE_SOURCE — remove the domain definition from
    /// the source after a successful migration. Without this the
    /// source keeps a (now-stopped) copy of the domain.
    pub undefine_source: bool,
    /// VIR_MIGRATE_AUTO_CONVERGE — let QEMU throttle the guest CPU
    /// when memory dirties faster than the network can transfer.
    /// Lets convergence happen on chatty workloads at the cost of
    /// brief guest stalls during the final iterations.
    pub auto_converge: bool,
    /// Optional bandwidth cap in MiB/s (per the libvirt-rs
    /// `Domain::migrate` `bandwidth` argument). 0 = unlimited.
    pub bandwidth_mibs: u64,
    /// Optional explicit URI the destination libvirtd should hand to
    /// QEMU for the wire-level migration channel (e.g. when the SSH
    /// tunnel terminates on a different IP than libvirtd's own).
    /// `None` = let libvirt pick.
    pub dest_uri: Option<String>,
    /// Optional dest XML override (libvirt-rs `Domain::migrate2`).
    /// Rewrites a few fields like network interface bridge names so a
    /// guest can plug into a differently-named bridge on the target.
    /// Not used in this v1 — kept here so the wire shape is forward-
    /// compatible with the richer migrate2/migrate3 APIs.
    pub dest_xml: Option<String>,
    /// Optional rename on the destination. Most operators keep the
    /// same name; useful when there's already a placeholder defined.
    pub dest_name: Option<String>,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            live: true,
            persist_dest: true,
            undefine_source: false,
            auto_converge: true,
            bandwidth_mibs: 0,
            dest_uri: None,
            dest_xml: None,
            dest_name: None,
        }
    }
}

impl MigrationConfig {
    /// Compose the libvirt migrate flag bitmask. Always includes
    /// PEER2PEER when LIVE is set (we use the simpler P2P RPC; non-P2P
    /// requires the client to maintain two libvirtd connections itself,
    /// which we already do but the P2P path has fewer failure modes).
    pub fn flags(&self) -> u32 {
        let mut f: u32 = 0;
        if self.live {
            f |= virt::sys::VIR_MIGRATE_LIVE;
            f |= virt::sys::VIR_MIGRATE_PEER2PEER;
        }
        if self.persist_dest {
            f |= virt::sys::VIR_MIGRATE_PERSIST_DEST;
        }
        if self.undefine_source {
            f |= virt::sys::VIR_MIGRATE_UNDEFINE_SOURCE;
        }
        if self.auto_converge {
            f |= virt::sys::VIR_MIGRATE_AUTO_CONVERGE;
        }
        f
    }
}

/// Phase of a migration job, as derived from VIR_DOMAIN_JOB_*.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MigrationPhase {
    /// No active job (or the prior one was already cleaned up).
    None,
    /// Active and bounded — libvirt knows the total transfer size.
    Bounded,
    /// Active and unbounded — total size unknown ahead of time
    /// (memory dirties faster than transfer, post-copy in flight, etc).
    Unbounded,
    Completed,
    Failed,
    Cancelled,
}

impl MigrationPhase {
    pub fn from_libvirt(t: i32) -> Self {
        // VIR_DOMAIN_JOB_* constants are stable over libvirt versions.
        match t {
            0 => Self::None,        // VIR_DOMAIN_JOB_NONE
            1 => Self::Bounded,     // VIR_DOMAIN_JOB_BOUNDED
            2 => Self::Unbounded,   // VIR_DOMAIN_JOB_UNBOUNDED
            3 => Self::Completed,   // VIR_DOMAIN_JOB_COMPLETED
            4 => Self::Failed,      // VIR_DOMAIN_JOB_FAILED
            5 => Self::Cancelled,   // VIR_DOMAIN_JOB_CANCELLED
            _ => Self::None,
        }
    }
}

/// Snapshot of an in-flight migration as observed via job stats. All
/// units are bytes / milliseconds for ergonomic UI consumption.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MigrationProgress {
    pub phase: Option<MigrationPhase>,
    pub data_total: Option<u64>,
    pub data_processed: Option<u64>,
    pub data_remaining: Option<u64>,
    pub mem_total: Option<u64>,
    pub mem_processed: Option<u64>,
    pub mem_remaining: Option<u64>,
    pub time_elapsed_ms: Option<u64>,
    pub time_remaining_ms: Option<u64>,
    pub downtime_ms: Option<u64>,
    pub error: Option<String>,
}

impl MigrationProgress {
    /// Map a libvirt-rs JobStats into our wire shape.
    pub fn from_job_stats(s: virt::domain::JobStats) -> Self {
        Self {
            phase: Some(MigrationPhase::from_libvirt(s.r#type)),
            data_total: s.data_total,
            data_processed: s.data_processed,
            data_remaining: s.data_remaining,
            mem_total: s.mem_total,
            mem_processed: s.mem_processed,
            mem_remaining: s.mem_remaining,
            time_elapsed_ms: s.time_elapsed,
            time_remaining_ms: s.time_remaining,
            downtime_ms: s.downtime,
            error: s.error_message,
        }
    }
}

/// Convert a libvirt::error::Error into our taxonomy with an operation
/// hint for the migration paths.
pub fn migrate_err(op: &str, e: virt::error::Error) -> VirtManagerError {
    VirtManagerError::OperationFailed {
        operation: op.into(),
        reason: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_flags_are_live_persist_p2p_converge() {
        let f = MigrationConfig::default().flags();
        assert!(f & virt::sys::VIR_MIGRATE_LIVE != 0);
        assert!(f & virt::sys::VIR_MIGRATE_PEER2PEER != 0);
        assert!(f & virt::sys::VIR_MIGRATE_PERSIST_DEST != 0);
        assert!(f & virt::sys::VIR_MIGRATE_AUTO_CONVERGE != 0);
        assert!(f & virt::sys::VIR_MIGRATE_UNDEFINE_SOURCE == 0);
    }

    #[test]
    fn undefine_source_flag_round_trips() {
        let cfg = MigrationConfig {
            undefine_source: true,
            ..MigrationConfig::default()
        };
        assert!(cfg.flags() & virt::sys::VIR_MIGRATE_UNDEFINE_SOURCE != 0);
    }

    #[test]
    fn live_off_drops_p2p_too() {
        let cfg = MigrationConfig {
            live: false,
            ..MigrationConfig::default()
        };
        assert_eq!(cfg.flags() & virt::sys::VIR_MIGRATE_LIVE, 0);
        assert_eq!(cfg.flags() & virt::sys::VIR_MIGRATE_PEER2PEER, 0);
    }

    #[test]
    fn phase_from_libvirt_known_values() {
        assert_eq!(MigrationPhase::from_libvirt(0), MigrationPhase::None);
        assert_eq!(MigrationPhase::from_libvirt(1), MigrationPhase::Bounded);
        assert_eq!(MigrationPhase::from_libvirt(2), MigrationPhase::Unbounded);
        assert_eq!(MigrationPhase::from_libvirt(3), MigrationPhase::Completed);
        assert_eq!(MigrationPhase::from_libvirt(4), MigrationPhase::Failed);
        assert_eq!(MigrationPhase::from_libvirt(5), MigrationPhase::Cancelled);
        assert_eq!(MigrationPhase::from_libvirt(99), MigrationPhase::None);
    }
}
