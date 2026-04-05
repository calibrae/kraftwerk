use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use virt::connect::Connect;
use virt::domain::Domain;
use virt::stream::Stream;
use virt::sys;

use crate::models::error::VirtManagerError;

/// VIR_STREAM_NONBLOCK — makes recv() return immediately when no data available.
const STREAM_NONBLOCK: sys::virStreamFlags = 1;

/// A bidirectional serial console session over a libvirt stream.
///
/// Data flows:
///   VM -> Stream::recv -> on_data callback -> (Tauri event to frontend)
///   Frontend -> send() -> Stream::send -> VM
///
/// The read loop runs on a dedicated thread and can be stopped via close().
pub struct ConsoleSession {
    stream: Arc<Stream>,
    running: Arc<AtomicBool>,
    read_thread: Option<thread::JoinHandle<()>>,
}

impl ConsoleSession {
    /// Open a console session for the named domain.
    ///
    /// `on_data` is called from a background thread whenever bytes arrive from the VM.
    pub fn open<F>(
        conn: &Connect,
        domain_name: &str,
        on_data: F,
    ) -> Result<Self, VirtManagerError>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let stream =
            Stream::new(conn, STREAM_NONBLOCK).map_err(|e| VirtManagerError::OperationFailed {
                operation: "streamNew".into(),
                reason: e.to_string(),
            })?;

        let domain = Domain::lookup_by_name(conn, domain_name).map_err(|_| {
            VirtManagerError::DomainNotFound {
                name: domain_name.to_string(),
            }
        })?;

        // VIR_DOMAIN_CONSOLE_FORCE = 1
        domain
            .open_console(None, &stream, sys::VIR_DOMAIN_CONSOLE_FORCE)
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "openConsole".into(),
                reason: e.to_string(),
            })?;

        let stream = Arc::new(stream);
        let running = Arc::new(AtomicBool::new(true));

        // Spawn non-blocking read loop
        let read_stream = stream.clone();
        let read_running = running.clone();
        let read_thread = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while read_running.load(Ordering::Relaxed) {
                match read_stream.recv(&mut buf) {
                    Ok(0) => {
                        // No data available (non-blocking), brief sleep
                        thread::sleep(Duration::from_millis(20));
                    }
                    Ok(n) => {
                        on_data(buf[..n].to_vec());
                    }
                    Err(e) => {
                        // -2 means would-block in libvirt, but virt crate maps it to Error
                        // Check if it's a real error or just no data
                        let msg = e.to_string();
                        if msg.contains("would block") || msg.contains("-2") {
                            thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        // Real error — stop the loop
                        log::warn!("Console read error: {msg}");
                        read_running.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }
        });

        log::info!("Opened console session for '{domain_name}'");

        Ok(ConsoleSession {
            stream,
            running,
            read_thread: Some(read_thread),
        })
    }

    /// Send bytes to the VM's serial console (keyboard input from the user).
    pub fn send(&self, data: &[u8]) -> Result<usize, VirtManagerError> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(VirtManagerError::OperationFailed {
                operation: "consoleSend".into(),
                reason: "Session is closed".into(),
            });
        }
        self.stream
            .send(data)
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "consoleSend".into(),
                reason: e.to_string(),
            })
    }

    /// Check if the session is still active.
    pub fn is_active(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Close the console session and stop the read thread.
    pub fn close(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.read_thread.take() {
            let _ = handle.join();
        }
        log::info!("Closed console session");
    }
}

impl Drop for ConsoleSession {
    fn drop(&mut self) {
        self.close();
    }
}
