//! SPICE proxy: SSH tunnel + capsaicin SPICE client bridge.
//!
//! Flow mirrors the VNC proxy but terminates the protocol in Rust using
//! `capsaicin-client`. The frontend receives decoded `ClientEvent`s as
//! Tauri events and sends `InputEvent`s back via a command.
//!
//! ```text
//!   qemu on hypervisor <-- ssh -L --> local:PORT <-- TCP --> SpiceClient
//!                                                              |
//!                                                              v
//!                                                   ClientEvent --> Tauri -> WebView
//!                                                   InputEvent  <-- Tauri <- WebView
//! ```

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use capsaicin_client::{ClientEvent, InputEvent, SpiceClient};
use regex::Regex;
use tokio::sync::mpsc;

use crate::models::error::VirtManagerError;

/// Parse the SPICE graphics endpoint from a domain XML.
///
/// Accepts both single- and double-quoted attributes, e.g.:
///   `<graphics type='spice' port='5902' listen='127.0.0.1'/>`
///
/// Returns `None` when there is no SPICE graphics element, when `port`
/// is absent, or when port is -1 (autoport not yet resolved).
pub fn parse_spice_endpoint(xml: &str) -> Option<(String, u16)> {
    static GFX_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"<graphics\s+([^>]*)type=['"]spice['"]([^>]*)>"#).unwrap()
    });
    let caps = GFX_RE.captures(xml)?;
    let attrs = format!("{}{}", caps.get(1)?.as_str(), caps.get(2)?.as_str());

    let port = attr(&attrs, "port")
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|&p| p > 0)
        .map(|p| p as u16)?;
    let listen = attr(&attrs, "listen").unwrap_or_else(|| "127.0.0.1".to_string());
    Some((listen, port))
}

/// Extract the optional SPICE password (`passwd='...'`).
pub fn parse_spice_password(xml: &str) -> Option<String> {
    static GFX_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"<graphics\s+([^>]*)type=['"]spice['"]([^>]*)>"#).unwrap()
    });
    let caps = GFX_RE.captures(xml)?;
    let attrs = format!("{}{}", caps.get(1)?.as_str(), caps.get(2)?.as_str());
    attr(&attrs, "passwd")
}

fn attr(haystack: &str, key: &str) -> Option<String> {
    let pat = format!(r#"{}=['"]([^'"]*)['"]"#, key);
    let re = Regex::new(&pat).ok()?;
    re.captures(haystack)?
        .get(1)
        .map(|m| m.as_str().to_string())
}

fn pick_local_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

/// An active SPICE session.
///
/// Holds the SSH tunnel child, the event receiver the embedder drains,
/// and the input sender the embedder pushes on.
pub struct SpiceSession {
    pub events_rx: mpsc::Receiver<ClientEvent>,
    pub input_tx: mpsc::Sender<InputEvent>,
    running: Arc<AtomicBool>,
    ssh_child: Option<Child>,
}

impl SpiceSession {
    /// Establish an SSH tunnel to the guest's SPICE port, then connect
    /// a capsaicin `SpiceClient` over it.
    pub fn start(
        ssh_target: &str,
        remote_listen: &str,
        remote_port: u16,
        password: &str,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Self, VirtManagerError> {
        let forward_port = pick_local_port().map_err(|e| VirtManagerError::OperationFailed {
            operation: "pickPort".into(),
            reason: e.to_string(),
        })?;

        let forward_arg = format!("127.0.0.1:{forward_port}:{remote_listen}:{remote_port}");
        log::info!("starting ssh tunnel for SPICE: {forward_arg} via {ssh_target}");
        let child = Command::new("ssh")
            .args([
                "-N",
                "-o", "ExitOnForwardFailure=yes",
                "-o", "ServerAliveInterval=15",
                "-L", &forward_arg,
                ssh_target,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "sshTunnel".into(),
                reason: format!("failed to spawn ssh: {e}"),
            })?;

        // Wait until the tunnel is connectable.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match std::net::TcpStream::connect_timeout(
                &format!("127.0.0.1:{forward_port}").parse().unwrap(),
                Duration::from_millis(250),
            ) {
                Ok(_) => break,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(150));
                }
                Err(e) => {
                    return Err(VirtManagerError::OperationFailed {
                        operation: "sshTunnelReady".into(),
                        reason: format!("SPICE tunnel did not come up: {e}"),
                    });
                }
            }
        }

        // Connect the SPICE client over the tunneled socket (inside runtime).
        let password = password.to_string();
        let client = runtime.block_on(async move {
            SpiceClient::connect(&format!("127.0.0.1:{forward_port}"), &password)
                .await
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "spiceConnect".into(),
                    reason: e.to_string(),
                })
        })?;

        // We need the input_tx and events_rx out of the client. The current
        // SpiceClient owns them; we consume it into its mailboxes via a pump task.
        let (events_tx, events_rx) = mpsc::channel::<ClientEvent>(256);
        let (input_tx, mut input_rx) = mpsc::channel::<InputEvent>(256);

        let running = Arc::new(AtomicBool::new(true));
        let run_running = running.clone();

        runtime.spawn(async move {
            let mut client = client;
            loop {
                tokio::select! {
                    evt = client.next_event() => {
                        match evt {
                            Some(e) => {
                                if events_tx.send(e).await.is_err() {
                                    break; // frontend is gone
                                }
                            }
                            None => break, // client closed
                        }
                    }
                    input = input_rx.recv() => {
                        match input {
                            Some(i) => {
                                if let Err(e) = client.send_input(i).await {
                                    log::warn!("SPICE input send failed: {e}");
                                }
                            }
                            None => {
                                // embedder dropped the input sender — treat as shutdown request
                                break;
                            }
                        }
                    }
                }
                if !run_running.load(Ordering::Relaxed) {
                    break;
                }
            }
            log::info!("SPICE pump task exiting");
        });

        Ok(SpiceSession {
            events_rx,
            input_tx,
            running,
            ssh_child: Some(child),
        })
    }

    pub fn close(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(mut child) = self.ssh_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for SpiceSession {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spice_endpoint_single_quotes() {
        let xml = "<domain><devices><graphics type='spice' port='5902' autoport='no' listen='127.0.0.1'/></devices></domain>";
        let (h, p) = parse_spice_endpoint(xml).unwrap();
        assert_eq!(h, "127.0.0.1");
        assert_eq!(p, 5902);
    }

    #[test]
    fn parses_spice_endpoint_double_quotes() {
        let xml = r#"<domain><graphics type="spice" port="5901" listen="0.0.0.0"/></domain>"#;
        let (h, p) = parse_spice_endpoint(xml).unwrap();
        assert_eq!(h, "0.0.0.0");
        assert_eq!(p, 5901);
    }

    #[test]
    fn defaults_spice_listen_when_missing() {
        let xml = "<domain><graphics type='spice' port='5902'/></domain>";
        let (h, _) = parse_spice_endpoint(xml).unwrap();
        assert_eq!(h, "127.0.0.1");
    }

    #[test]
    fn rejects_port_minus_one() {
        let xml = "<domain><graphics type='spice' port='-1' autoport='yes'/></domain>";
        assert!(parse_spice_endpoint(xml).is_none());
    }

    #[test]
    fn rejects_vnc_endpoint() {
        let xml = "<domain><graphics type='vnc' port='5900'/></domain>";
        assert!(parse_spice_endpoint(xml).is_none());
    }

    #[test]
    fn parses_spice_password() {
        let xml = "<domain><graphics type='spice' port='5902' passwd='secret123'/></domain>";
        assert_eq!(parse_spice_password(xml).as_deref(), Some("secret123"));
    }

    #[test]
    fn no_password_returns_none() {
        let xml = "<domain><graphics type='spice' port='5902'/></domain>";
        assert!(parse_spice_password(xml).is_none());
    }
}
