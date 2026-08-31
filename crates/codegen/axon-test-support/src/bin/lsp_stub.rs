//! A minimal LSP server over stdio: enough handshake to be real, nothing more.
//!
//! Exists so a test can put a *genuine external process with a live socket* on
//! the other end of `LspBackendAdapter` without depending on rust-analyzer or
//! any other real language server being installed. That distinction matters:
//! the teardown race this stub was written for (v0.3.7's `Sender is alive`
//! abort) lives entirely in `async-lsp`'s main loop, so a mock that never
//! spawns a process cannot reach it. The dummy `LspBackend` impls elsewhere in
//! the tree are the wrong tool for the same reason.
//!
//! Speaks only what `LspClient::start` and `LspClient::shutdown` require:
//! `initialize` → capabilities, `initialized` → ignored, `shutdown` → null,
//! `exit` → quit. Any other request gets a null result rather than silence, so
//! a caller waiting on a response never hangs on an unexpected method.
//!
//! Knobs, for the teardown tests:
//! - `LSP_STUB_IGNORE_SHUTDOWN=1` — accept `shutdown`/`exit` and answer
//!   neither, so the caller's own bound is the only thing that ends the wait.
//! - `LSP_STUB_READY_FILE=<path>` — touch `<path>` once `initialize` has been
//!   answered, so a test can wait for "the server is really up" instead of
//!   sleeping and hoping.

use std::io::{Read, Write as _};

/// Read one `Content-Length`-framed message. `None` on a clean EOF.
fn read_message(stdin: &mut impl Read) -> Option<serde_json::Value> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        match stdin.read(&mut byte) {
            Ok(0) | Err(_) => return None, // EOF, or the pipe died with us
            Ok(_) => header.push(byte[0]),
        }
    }

    // Only Content-Length matters; Content-Type is optional and ignored.
    let header = String::from_utf8_lossy(&header);
    let len: usize = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .expect("LSP frame without a parseable Content-Length");

    let mut body = vec![0u8; len];
    stdin.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn send(message: &serde_json::Value) {
    let body = serde_json::to_vec(message).expect("serialize response");
    let mut stdout = std::io::stdout().lock();
    // The framing is the protocol: header, blank line, exact byte count.
    write!(stdout, "Content-Length: {}\r\n\r\n", body.len()).expect("write header");
    stdout.write_all(&body).expect("write body");
    // Unflushed, the client waits forever and the test times out looking like
    // a teardown hang rather than a stub bug.
    stdout.flush().expect("flush");
}

fn reply(id: &serde_json::Value, result: serde_json::Value) {
    send(&serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}));
}

fn main() {
    let ignore_shutdown = std::env::var_os("LSP_STUB_IGNORE_SHUTDOWN").is_some();
    let ready_file = std::env::var_os("LSP_STUB_READY_FILE");
    let mut stdin = std::io::stdin().lock();

    while let Some(message) = read_message(&mut stdin) {
        let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");
        // A request carries an id and needs an answer; a notification does not.
        let id = message.get("id");

        match (method, id) {
            ("initialize", Some(id)) => {
                // An empty capability set is a legal server: it advertises
                // support for nothing, which is all this stub can honour.
                reply(id, serde_json::json!({"capabilities": {}}));
                if let Some(path) = &ready_file {
                    let _ = std::fs::write(path, b"ready");
                }
            }
            ("shutdown", Some(id)) if !ignore_shutdown => reply(id, serde_json::Value::Null),
            ("shutdown", _) => {} // deliberately unanswered
            ("exit", _) if !ignore_shutdown => return,
            ("exit", _) => {} // deliberately still alive
            (_, Some(id)) => reply(id, serde_json::Value::Null),
            (_, None) => {} // an unknown notification is not an error
        }
    }
}
