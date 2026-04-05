use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use virt::connect::Connect;
use virt::domain::Domain;
use virt::stream::Stream;
use virt::sys;

use crate::models::error::VirtManagerError;

/// A bidirectional serial console session over a libvirt stream.
///
/// Uses a blocking stream on a dedicated read thread. Data arrives
/// as soon as the VM sends it — no polling delay.
pub struct ConsoleSession {
    stream: Arc<Stream>,
    running: Arc<AtomicBool>,
    read_thread: Option<thread::JoinHandle<()>>,
}

impl ConsoleSession {
    pub fn open<F>(
        conn: &Connect,
        domain_name: &str,
        on_data: F,
    ) -> Result<Self, VirtManagerError>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        // Blocking stream — recv() will block until data arrives
        let stream =
            Stream::new(conn, 0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "streamNew".into(),
                reason: e.to_string(),
            })?;

        let domain = Domain::lookup_by_name(conn, domain_name).map_err(|_| {
            VirtManagerError::DomainNotFound {
                name: domain_name.to_string(),
            }
        })?;

        domain
            .open_console(None, &stream, sys::VIR_DOMAIN_CONSOLE_FORCE)
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "openConsole".into(),
                reason: e.to_string(),
            })?;

        let stream = Arc::new(stream);
        let running = Arc::new(AtomicBool::new(true));

        let read_stream = stream.clone();
        let read_running = running.clone();
        let read_thread = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while read_running.load(Ordering::Relaxed) {
                // Blocking recv — wakes as soon as bytes arrive from the VM
                let ret = unsafe {
                    sys::virStreamRecv(
                        read_stream.as_ptr(),
                        buf.as_mut_ptr() as *mut libc::c_char,
                        buf.len(),
                    )
                };

                if ret > 0 {
                    on_data(buf[..ret as usize].to_vec());
                } else if ret == 0 {
                    // EOF
                    eprintln!("Console stream EOF");
                    read_running.store(false, Ordering::Relaxed);
                    break;
                } else {
                    // ret < 0: error
                    eprintln!("Console read error (ret={ret})");
                    read_running.store(false, Ordering::Relaxed);
                    break;
                }
            }
        });

        eprintln!("Opened console session for '{domain_name}'");

        Ok(ConsoleSession {
            stream,
            running,
            read_thread: Some(read_thread),
        })
    }

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

    pub fn is_active(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn close(&mut self) {
        if !self.running.swap(false, Ordering::Relaxed) {
            return; // already closed
        }
        // Abort the stream to unblock the read thread
        unsafe {
            sys::virStreamAbort(self.stream.as_ptr());
        }
        if let Some(handle) = self.read_thread.take() {
            let _ = handle.join();
        }
        eprintln!("Closed console session");
    }
}

impl Drop for ConsoleSession {
    fn drop(&mut self) {
        self.close();
    }
}
