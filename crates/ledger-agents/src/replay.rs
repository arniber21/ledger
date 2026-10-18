//! Local HTTP server that replays recorded Gemini `generateContent` bodies.
//!
//! The bodies were captured from the live API and contain no API key. Rig's
//! Gemini client sends the real request shape; this server only chooses which
//! saved body to return.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

/// Binds `127.0.0.1:0` and serves fixture bodies. Returns the API root.
pub fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("replay bind");
    let address = listener.local_addr().expect("replay address");
    listener
        .set_nonblocking(false)
        .expect("replay blocking mode");
    thread::spawn(move || {
        for _ in 0..32 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let request = read_request(&mut stream).unwrap_or_default();
            let body = body_for(&request);
            let header = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
    format!("http://{address}")
}

fn body_for(request: &str) -> String {
    // A later turn carries functionResponse and closes with text.
    // The approver's double-pay prompt asks to approve the settled bill. The
    // payer's double-pay prompt asks to pay it. Those sentences are the ones
    // `approver_prompt` and `payer_prompt` append while the fault is on.
    // The clean payer prompt is the only other request that says "Pay the open bill".
    // The approver's tool text mentions pay_bill and must not take that body.
    let name = if request.contains("functionResponse") {
        "text.json"
    } else if request.contains("Also call approve_bill once with the settled bill id") {
        "approve-settled-call.json"
    } else if request.contains("Also call pay_bill once with the settled bill id") {
        "pay-settled-call.json"
    } else if request.contains("Pay the open bill") {
        "pay-bill-call.json"
    } else {
        "text.json"
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/gemini")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("fixture {}: {err}", path.display()))
}

fn read_request(stream: &mut impl Read) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 2048];
    loop {
        let read = match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(count) => count,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => break,
            Err(err) => return Err(err),
        };
        buf.extend_from_slice(&tmp[..read]);
        if let Some(header_end) = find_subslice(&buf, b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&buf[..header_end]);
            let length = content_length(&header).unwrap_or(0);
            let body_at = header_end + 4;
            while buf.len() < body_at + length {
                let read = match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(count) => count,
                    Err(err) if err.kind() == std::io::ErrorKind::TimedOut => break,
                    Err(err) => return Err(err),
                };
                buf.extend_from_slice(&tmp[..read]);
            }
            break;
        }
        if buf.len() > 2_000_000 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn content_length(header: &str) -> Option<usize> {
    header.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
