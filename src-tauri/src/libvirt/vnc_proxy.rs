//! WebSocket ↔ VNC bridge with SSH tunnel fallback.
//!
//! Remote libvirt (qemu+ssh://) cannot pass file descriptors through the
//! RPC channel, so we use the virt-viewer approach:
//!   1. Parse the domain XML for the VNC listen address + port.
//!   2. Parse the libvirt URI for the SSH user@host.
//!   3. Spawn `ssh -N -L 127.0.0.1:LOCAL:LISTEN:PORT user@host` as a child
//!      process to tunnel the VNC TCP socket to localhost.
//!   4. Start a local WebSocket listener that bridges WS ↔ tunneled TCP.
//!   5. The frontend's noVNC client connects to ws://127.0.0.1:<local>.

use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

use crate::models::error::VirtManagerError;

/// Parse a libvirt URI for its SSH target (user@host).
///
/// Accepts:
///   qemu+ssh://user@host/system
///   qemu+ssh://host/system
///   qemu+ssh://user@host:22/system
pub fn parse_ssh_target(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("qemu+ssh://")?;
    let authority = rest.split('/').next()?;
    if authority.is_empty() {
        return None;
    }
    Some(authority.to_string())
}

/// Parse VNC host+port from domain XML.
/// Returns ("127.0.0.1", 5900) style tuple for the listen-address + port.
pub fn parse_vnc_endpoint(xml: &str) -> Option<(String, u16)> {
    // Match <graphics type='vnc' ... port='N' ... listen='HOST' ...>
    // Quotes may be single or double; attributes may be in any order.
    use regex::Regex;
    static GFX_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"<graphics\s+([^>]*)type=['"]vnc['"]([^>]*)>"#).unwrap()
    });
    let caps = GFX_RE.captures(xml)?;
    // Concatenate the two capture groups (before and after type=...)
    let attrs_full = format!("{}{}", caps.get(1)?.as_str(), caps.get(2)?.as_str());

    let port = attr(&attrs_full, "port")
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|&p| p > 0)
        .map(|p| p as u16)?;
    let listen = attr(&attrs_full, "listen").unwrap_or_else(|| "127.0.0.1".to_string());
    Some((listen, port))
}

fn attr(haystack: &str, key: &str) -> Option<String> {
    let pat = format!(r#"{}=['"]([^'"]*)['"]"#, key);
    let re = regex::Regex::new(&pat).ok()?;
    re.captures(haystack)?
        .get(1)
        .map(|m| m.as_str().to_string())
}

/// Allocate an unused local TCP port. Binds 127.0.0.1:0, reads the port, closes.
/// There's a tiny race here (port could be taken before `ssh` binds it) but it's
/// the standard pattern.
fn pick_local_port() -> std::io::Result<u16> {
    let l = StdTcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

/// An active VNC proxy session.
pub struct VncSession {
    pub port: u16,
    running: Arc<AtomicBool>,
    ssh_child: Option<Child>,
}

impl VncSession {
    /// Start a VNC tunnel + WebSocket bridge.
    ///
    /// `ssh_target` — user@host for the SSH subprocess
    /// `remote_listen` — VNC listen address inside the hypervisor (usually "127.0.0.1")
    /// `remote_port` — VNC port on the hypervisor
    pub fn start(
        ssh_target: &str,
        remote_listen: &str,
        remote_port: u16,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Self, VirtManagerError> {
        // 1. Pick a free local port for SSH to forward to
        let forward_port = pick_local_port().map_err(|e| VirtManagerError::OperationFailed {
            operation: "pickPort".into(),
            reason: e.to_string(),
        })?;

        // 2. Spawn ssh -N -L forward_port:remote_listen:remote_port ssh_target
        let forward_arg = format!("127.0.0.1:{forward_port}:{remote_listen}:{remote_port}");
        log::info!("starting ssh tunnel: {forward_arg} via {ssh_target}");
        let child = Command::new("ssh")
            .args([
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "StreamLocalBindUnlink=yes",
                "-L",
                &forward_arg,
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

        // 3. Wait for the tunnel to become ready by polling TCP connect.
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
                        reason: format!("tunnel did not come up: {e}"),
                    });
                }
            }
        }

        // 4. Bind our WebSocket listener on another local port
        let running = Arc::new(AtomicBool::new(true));
        let (ws_port, listener) = runtime.block_on(async {
            let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
            let listener = TcpListener::bind(addr).await.map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "vncBind".into(),
                    reason: e.to_string(),
                }
            })?;
            let port = listener.local_addr().map_err(|e| VirtManagerError::OperationFailed {
                operation: "vncLocalAddr".into(),
                reason: e.to_string(),
            })?.port();
            Ok::<_, VirtManagerError>((port, listener))
        })?;

        // 5. Spawn the proxy task
        let run_running = running.clone();
        runtime.spawn(async move {
            let _ = run_proxy(listener, forward_port, run_running).await;
        });

        log::info!("VNC proxy ready: ws://127.0.0.1:{ws_port}");
        Ok(VncSession {
            port: ws_port,
            running,
            ssh_child: Some(child),
        })
    }

    pub fn close(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(mut child) = self.ssh_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for VncSession {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(mut child) = self.ssh_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn run_proxy(
    listener: TcpListener,
    forward_port: u16,
    running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Accept one WS client
    let (tcp, _peer) = listener.accept().await?;
    drop(listener);

    let ws = tokio_tungstenite::accept_async(tcp).await?;
    let (mut ws_sink, mut ws_stream) = ws.split();

    // Connect to the SSH-tunneled VNC endpoint
    let vnc = TcpStream::connect(("127.0.0.1", forward_port)).await?;
    let (mut vnc_read, mut vnc_write) = vnc.into_split();

    let running_a = running.clone();
    let vnc_to_ws = async move {
        let mut buf = [0u8; 8192];
        while running_a.load(Ordering::Relaxed) {
            match vnc_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if ws_sink.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = ws_sink.close().await;
    };

    let running_b = running.clone();
    let ws_to_vnc = async move {
        while running_b.load(Ordering::Relaxed) {
            match ws_stream.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    if vnc_write.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Text(t))) => {
                    if vnc_write.write_all(t.as_bytes()).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            }
        }
        let _ = vnc_write.shutdown().await;
    };

    tokio::join!(vnc_to_ws, ws_to_vnc);
    running.store(false, Ordering::Relaxed);
    log::info!("VNC proxy closed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_uri_with_user() {
        assert_eq!(
            parse_ssh_target("qemu+ssh://testuser@testhost/system"),
            Some("testuser@testhost".into()),
        );
    }

    #[test]
    fn parses_ssh_uri_without_user() {
        assert_eq!(parse_ssh_target("qemu+ssh://testhost/system"), Some("testhost".into()));
    }

    #[test]
    fn parses_ssh_uri_with_port() {
        assert_eq!(
            parse_ssh_target("qemu+ssh://testuser@testhost:2222/system"),
            Some("testuser@testhost:2222".into()),
        );
    }

    #[test]
    fn rejects_non_ssh_uri() {
        assert_eq!(parse_ssh_target("qemu:///system"), None);
        assert_eq!(parse_ssh_target(""), None);
    }

    #[test]
    fn parses_vnc_endpoint_from_xml() {
        let xml = r#"<domain><devices><graphics type='vnc' port='5901' autoport='no' listen='127.0.0.1'/></devices></domain>"#;
        let (host, port) = parse_vnc_endpoint(xml).unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 5901);
    }

    #[test]
    fn parses_vnc_endpoint_double_quotes() {
        let xml = r#"<domain><graphics type="vnc" port="5902" listen="0.0.0.0"/></domain>"#;
        let (host, port) = parse_vnc_endpoint(xml).unwrap();
        assert_eq!(host, "0.0.0.0");
        assert_eq!(port, 5902);
    }

    #[test]
    fn parses_vnc_endpoint_default_listen() {
        let xml = r#"<domain><graphics type='vnc' port='5903'/></domain>"#;
        let (host, port) = parse_vnc_endpoint(xml).unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 5903);
    }

    #[test]
    fn rejects_port_minus_one() {
        // autoport='yes' with port=-1 means the port isn't assigned yet
        let xml = r#"<domain><graphics type='vnc' port='-1' autoport='yes'/></domain>"#;
        assert!(parse_vnc_endpoint(xml).is_none());
    }

    #[test]
    fn rejects_spice_endpoint() {
        let xml = r#"<domain><graphics type='spice' port='5900'/></domain>"#;
        assert!(parse_vnc_endpoint(xml).is_none());
    }

    #[test]
    fn pick_local_port_returns_nonzero() {
        let p = pick_local_port().unwrap();
        assert!(p > 0);
    }
}
