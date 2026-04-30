//! Trust-on-first-use host key check for libvirt+ssh connections.
//!
//! Runs `ssh-keyscan` to fetch the remote host key, compares against
//! the local `~/.ssh/known_hosts` via `ssh-keygen -F`, and exposes
//! `accept` to append the new key. The frontend surfaces this in a
//! dialog before opening the libvirt connection so we never hand off
//! to libvirt's ssh-without-a-TTY (which silently hangs or refuses).

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;

use crate::models::error::VirtManagerError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HostKeyStatus {
    Trusted,
    New,
    Changed,
    Unreachable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub status: HostKeyStatus,
    pub keyscan_line: Option<String>,
    pub fingerprint: Option<String>,
    pub key_type: Option<String>,
}

fn is_safe_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

pub fn check_host_key(host: &str, port: u16) -> Result<HostKeyInfo, VirtManagerError> {
    if !is_safe_host(host) {
        return Err(VirtManagerError::OperationFailed {
            operation: "checkHostKey".into(),
            reason: format!("invalid hostname: {host:?}"),
        });
    }

    let mut child = Command::new("ssh-keyscan")
        .arg("-T").arg("5")
        .arg("-p").arg(port.to_string())
        .arg("-t").arg("ed25519,ecdsa,rsa")
        .arg(host)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| VirtManagerError::OperationFailed {
            operation: "spawnKeyscan".into(),
            reason: e.to_string(),
        })?;
    let status = match child.wait_timeout(Duration::from_secs(8)) {
        Ok(Some(s)) => s,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(HostKeyInfo {
                host: host.to_string(), port,
                status: HostKeyStatus::Unreachable,
                keyscan_line: None, fingerprint: None, key_type: None,
            });
        }
        Err(e) => return Err(VirtManagerError::OperationFailed {
            operation: "keyscanWait".into(), reason: e.to_string(),
        }),
    };
    if !status.success() {
        return Ok(HostKeyInfo {
            host: host.to_string(), port,
            status: HostKeyStatus::Unreachable,
            keyscan_line: None, fingerprint: None, key_type: None,
        });
    }
    let mut stdout = String::new();
    if let Some(mut s) = child.stdout.take() { let _ = s.read_to_string(&mut stdout); }

    let keyscan_line = stdout
        .lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|s| s.to_string());
    let Some(kline) = keyscan_line else {
        return Ok(HostKeyInfo {
            host: host.to_string(), port,
            status: HostKeyStatus::Unreachable,
            keyscan_line: None, fingerprint: None, key_type: None,
        });
    };

    let mut parts = kline.split_whitespace();
    let _scanned_host = parts.next();
    let key_type = parts.next().map(|s| s.to_string());
    let key_b64 = parts.next();
    let fingerprint = key_b64.and_then(|b64| {
        use base64::{engine::general_purpose::{STANDARD, STANDARD_NO_PAD}, Engine as _};
        let decoded = STANDARD.decode(b64.trim()).ok()?;
        let mut h = Sha256::new();
        h.update(&decoded);
        Some(format!("SHA256:{}", STANDARD_NO_PAD.encode(h.finalize())))
    });

    let mut khpath = dirs::home_dir().unwrap_or_default();
    khpath.push(".ssh");
    khpath.push("known_hosts");
    let kh_arg = if port == 22 { host.to_string() } else { format!("[{host}]:{port}") };
    let mut child = Command::new("ssh-keygen")
        .arg("-F").arg(&kh_arg)
        .arg("-f").arg(&khpath)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| VirtManagerError::OperationFailed {
            operation: "spawnKeygen".into(), reason: e.to_string(),
        })?;
    let status = child.wait_timeout(Duration::from_secs(5))
        .map_err(|e| VirtManagerError::OperationFailed { operation: "keygenWait".into(), reason: e.to_string() })?;
    let mut keygen_stdout = String::new();
    if let Some(mut s) = child.stdout.take() { let _ = s.read_to_string(&mut keygen_stdout); }
    let trusted = match status {
        Some(s) if s.success() => {
            let scanned_body = kline.split_whitespace().nth(2);
            let stored_body = keygen_stdout
                .lines()
                .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .and_then(|l| l.split_whitespace().nth(2));
            scanned_body.is_some() && scanned_body == stored_body
        }
        _ => false,
    };
    let stored_any = keygen_stdout
        .lines()
        .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let result_status = if trusted {
        HostKeyStatus::Trusted
    } else if stored_any {
        HostKeyStatus::Changed
    } else {
        HostKeyStatus::New
    };

    Ok(HostKeyInfo {
        host: host.to_string(), port,
        status: result_status,
        keyscan_line: Some(kline),
        fingerprint,
        key_type,
    })
}

pub fn append_host_key(keyscan_line: &str) -> Result<(), VirtManagerError> {
    if keyscan_line.contains('\n') || keyscan_line.contains('\r') {
        return Err(VirtManagerError::OperationFailed {
            operation: "appendHostKey".into(),
            reason: "multi-line input rejected".into(),
        });
    }
    let parts: Vec<&str> = keyscan_line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(VirtManagerError::OperationFailed {
            operation: "appendHostKey".into(),
            reason: "expected `<host> <type> <base64>` line".into(),
        });
    }
    let mut khpath = dirs::home_dir().ok_or_else(|| VirtManagerError::OperationFailed {
        operation: "appendHostKey".into(),
        reason: "no $HOME".into(),
    })?;
    khpath.push(".ssh");
    if !khpath.exists() {
        std::fs::create_dir_all(&khpath).map_err(|e| VirtManagerError::OperationFailed {
            operation: "createSshDir".into(), reason: e.to_string(),
        })?;
    }
    khpath.push("known_hosts");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&khpath)
        .map_err(|e| VirtManagerError::OperationFailed {
            operation: "openKnownHosts".into(), reason: e.to_string(),
        })?;
    let mut payload = keyscan_line.trim_end().to_string();
    payload.push('\n');
    f.write_all(payload.as_bytes()).map_err(|e| VirtManagerError::OperationFailed {
        operation: "writeKnownHosts".into(), reason: e.to_string(),
    })?;
    Ok(())
}

pub fn forget_host_key(host: &str, port: u16) -> Result<(), VirtManagerError> {
    if !is_safe_host(host) {
        return Err(VirtManagerError::OperationFailed {
            operation: "forgetHostKey".into(),
            reason: "invalid host".into(),
        });
    }
    let mut khpath = dirs::home_dir().ok_or_else(|| VirtManagerError::OperationFailed {
        operation: "forgetHostKey".into(), reason: "no $HOME".into(),
    })?;
    khpath.push(".ssh");
    khpath.push("known_hosts");
    let arg = if port == 22 { host.to_string() } else { format!("[{host}]:{port}") };
    let r = Command::new("ssh-keygen")
        .arg("-R").arg(&arg)
        .arg("-f").arg(&khpath)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| VirtManagerError::OperationFailed {
            operation: "spawnKeygenR".into(), reason: e.to_string(),
        })?;
    if !r.status.success() {
        return Err(VirtManagerError::OperationFailed {
            operation: "keygenR".into(),
            reason: String::from_utf8_lossy(&r.stderr).trim().to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_hosts() {
        assert!(is_safe_host("example.com"));
        assert!(is_safe_host("host-1.lan"));
        assert!(!is_safe_host(""));
        assert!(!is_safe_host("a;rm -rf /"));
        assert!(!is_safe_host("[fe80::1]"));
        assert!(!is_safe_host("a b"));
    }

    #[test]
    fn append_rejects_multiline() {
        let r = append_host_key("foo ed25519 abc\nevil line");
        assert!(r.is_err());
    }
}
