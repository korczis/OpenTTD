//! RESEARCH-ONLY test support: a stand-in for a Majordomus server's `POST /mcp`, enough for the board
//! client (src/board.rs) — every request answered with one session id and an empty result, or with
//! the status a test sets — and stopped for real (its port refuses afterwards), so a test can watch
//! a daemon outlive its server, and not. Used by board.rs's tests and tests/daemons.rs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct FakeMcp {
    pub url: String,
    bodies: Arc<Mutex<Vec<String>>>,
    status: Arc<AtomicU16>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeMcp {
    /// Listen on a free loopback port and answer until stopped.
    pub fn start() -> FakeMcp {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        listener.set_nonblocking(true).expect("a non-blocking listener");
        let url = format!("http://{}", listener.local_addr().expect("its address"));
        let (bodies, status, stop) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicU16::new(200)), Arc::new(AtomicBool::new(false)));
        let (b, st, sp) = (bodies.clone(), status.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            while !sp.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        let _ = conn.set_nonblocking(false);
                        let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
                        let mut reader = BufReader::new(conn);
                        let mut len = 0usize;
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" { break; }
                            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") { len = v.trim().parse().unwrap_or(0); }
                        }
                        let mut body = vec![0u8; len];
                        let _ = reader.read_exact(&mut body);
                        b.lock().unwrap().push(String::from_utf8_lossy(&body).into_owned());
                        let code = st.load(Ordering::SeqCst);
                        let text = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
                        let reply = format!("HTTP/1.1 {code} X\r\nMcp-Session-Id: fake-1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
                        let _ = reader.into_inner().write_all(reply.as_bytes());
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        FakeMcp { url, bodies, status, stop, handle: Some(handle) }
    }

    /// Answer every later request with this HTTP status.
    pub fn set_status(&self, code: u16) {
        self.status.store(code, Ordering::SeqCst);
    }

    /// Every request body so far, oldest first.
    pub fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }

    /// Stop answering and close the port.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for FakeMcp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() { let _ = h.join(); }
    }
}
