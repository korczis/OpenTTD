//! RESEARCH-ONLY, tests only. A stand-in HTTP server for a test that reads another process over
//! HTTP (a running panel's /api/db/named): it answers each request with the next of its answers and
//! hands back what it was sent.

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

/// What one request carried: its request line (`POST /api/db/named HTTP/1.1`) and its JSON body
/// (null when it had none).
pub type Sent = (String, Value);

/// Serve `answers` ((status, JSON body) each, one request each, in order) on 127.0.0.1: the base
/// URL, and the handle whose `join` gives every request it was sent.
pub fn serve(answers: Vec<(u16, Value)>) -> (String, JoinHandle<Vec<Sent>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let h = std::thread::spawn(move || answers.into_iter().map(|(code, body)| {
        let (mut c, _) = l.accept().unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let (head, start, end) = loop {
            let n = c.read(&mut chunk).unwrap();
            buf.extend_from_slice(&chunk[..n]);
            if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..at]).to_string();
                let len = head.lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
                break (head, at + 4, at + 4 + len);
            }
        };
        while buf.len() < end {
            let n = c.read(&mut chunk).unwrap();
            buf.extend_from_slice(&chunk[..n]);
        }
        let sent: Value = serde_json::from_slice(&buf[start..end]).unwrap_or(Value::Null);
        let text = body.to_string();
        write!(c, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
               text.len()).unwrap();
        (head.lines().next().unwrap_or("").to_string(), sent)
    }).collect());
    (url, h)
}
