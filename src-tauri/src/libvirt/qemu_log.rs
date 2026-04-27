//! Read /var/log/libvirt/qemu/<vm>.log over SSH for the active connection.
//!
//! libvirt itself doesn't expose the qemu wrapper log via API (only the
//! per-domain `<log>` element points at the file path on the
//! hypervisor). To surface it in kraftwerk we re-use the same SSH target
//! parsing as the VNC/SPICE tunnels and run a one-shot `tail -n N` on
//! the remote side.
//!
//! Local-only `qemu:///system` connections also work — we drop the SSH
//! step and read the file directly.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;

use crate::libvirt::vnc_proxy::parse_ssh_target;
use crate::models::error::VirtManagerError;

/// Validate a VM name against libvirt's accepted character set so we can
/// safely interpolate it into a remote shell command.
pub fn is_safe_vm_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Read the last `lines` lines of /var/log/libvirt/qemu/<vm>.log.
/// `uri` is the libvirt URI of the active connection; if it's `qemu+ssh`
/// we dispatch over SSH, otherwise read locally.
pub fn read_qemu_log(uri: &str, vm_name: &str, lines: u32) -> Result<String, VirtManagerError> {
    if !is_safe_vm_name(vm_name) {
        return Err(VirtManagerError::OperationFailed {
            operation: "qemuLog".into(),
            reason: "invalid VM name".into(),
        });
    }
    let path = format!("/var/log/libvirt/qemu/{vm_name}.log");
    let lines = lines.clamp(1, 5000);

    if let Some(target) = parse_ssh_target(uri) {
        // Hard 15s wall-clock timeout: BatchMode + ConnectTimeout don't
        // always cover GUI-launched contexts where ssh-agent or known_hosts
        // can stall. We kill the child unconditionally past the budget.
        let remote_cmd = format!("tail -n {} {}", lines, shell_escape(&path));
        let mut child = Command::new("ssh")
            .arg("-o").arg("BatchMode=yes")
            .arg("-o").arg("ConnectTimeout=5")
            .arg("-o").arg("StrictHostKeyChecking=accept-new")
            .arg(&target)
            .arg(&remote_cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "qemuLogSpawnSsh".into(),
                reason: e.to_string(),
            })?;
        let status = match child.wait_timeout(Duration::from_secs(15)) {
            Ok(Some(s)) => s,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(VirtManagerError::OperationFailed {
                    operation: "qemuLogSshTimeout".into(),
                    reason: format!("ssh to {} timed out after 15s; check known_hosts and ssh-agent", target),
                });
            }
            Err(e) => {
                let _ = child.kill();
                return Err(VirtManagerError::OperationFailed {
                    operation: "qemuLogSshWait".into(),
                    reason: e.to_string(),
                });
            }
        };
        let mut stdout = String::new();
        if let Some(mut s_out) = child.stdout.take() { let _ = s_out.read_to_string(&mut stdout); }
        let mut stderr = String::new();
        if let Some(mut s_err) = child.stderr.take() { let _ = s_err.read_to_string(&mut stderr); }
        if !status.success() {
            return Err(VirtManagerError::OperationFailed {
                operation: "qemuLogSshTail".into(),
                reason: if stderr.trim().is_empty() {
                    format!("ssh exited with {:?}", status.code())
                } else {
                    stderr.trim().to_string()
                },
            });
        }
        Ok(stdout)
    } else {
        // Local read.
        std::fs::read_to_string(&path).map_err(|e| VirtManagerError::OperationFailed {
            operation: "qemuLogRead".into(),
            reason: format!("{path}: {e}"),
        }).map(|full| {
            // Manual tail since fs::read returns the whole file.
            let lines_vec: Vec<&str> = full.lines().collect();
            let start = lines_vec.len().saturating_sub(lines as usize);
            lines_vec[start..].join("\n")
        })
    }
}

/// Single-quote-wrap a shell argument. Replaces ' with '\'' inside.
fn shell_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_names() {
        assert!(is_safe_vm_name("good-vm"));
        assert!(is_safe_vm_name("vm_1.0"));
        assert!(!is_safe_vm_name(""));
        assert!(!is_safe_vm_name("a; rm -rf /"));
        assert!(!is_safe_vm_name("../etc/passwd"));
        assert!(!is_safe_vm_name("name with space"));
    }

    #[test]
    fn shell_escape_wraps_in_quotes() {
        assert_eq!(shell_escape("/var/log/x.log"), "'/var/log/x.log'");
    }

    #[test]
    fn shell_escape_handles_internal_quote() {
        assert_eq!(shell_escape("a'b"), "'a'\\''b'");
    }
}
