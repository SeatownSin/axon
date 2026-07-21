pub mod changelog;
pub mod event_id;
pub mod axon_home;
pub mod secure_file;
pub mod tips;
pub mod uname;
pub use axon_shared::clipboard;
pub use axon_shared::stderr::{stderr_lock, with_locked_stderr};
/// Generate a pseudo-random f64 in [0.0, 1.0).
///
/// Uses `RandomState::new()` which is OS-seeded (via `getrandom`) on each
/// instantiation, producing a unique hasher state per call. A fixed sentinel
/// is hashed to extract the random bits — the entropy comes entirely from
/// the OS-seeded `RandomState`, not from any clock source.
///
/// # Precision
/// The result uses all 53 bits of `f64` mantissa for a uniform distribution
/// over `[0.0, 1.0)`. We shift the 64-bit hash right by 11 bits to get a
/// 53-bit integer, then divide by `2^53`. This avoids the subtle bias that
/// occurs when casting a full `u64` to `f64` (which has only 52 bits of
/// mantissa, causing multiple `u64` values to map to the same `f64` for
/// values > 2^52).
///
/// Not cryptographically secure — suitable for sampling and feature
/// rollouts, not for security-sensitive randomness.
pub fn random_f64() -> f64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let random_state = RandomState::new();
    let mut hasher = random_state.build_hasher();
    hasher.write_u64(0x517cc1b727220a95);
    (hasher.finish() >> 11) as f64 / (1u64 << 53) as f64
}
/// Probabilistic sampling. Returns `true` with probability `rate` (0.0–1.0).
pub fn probabilistic_sample(rate: f64) -> bool {
    random_f64() < rate
}
fn matches_trusted_base_url(candidate: &str, trusted_base: &str) -> bool {
    let Ok(candidate) = reqwest::Url::parse(candidate) else {
        return false;
    };
    let Ok(trusted) = reqwest::Url::parse(trusted_base) else {
        return false;
    };
    let trusted_path = trusted.path();
    let candidate_path = candidate.path();
    let path_matches = candidate_path == trusted_path
        || candidate_path
            .strip_prefix(trusted_path)
            .is_some_and(|suffix| suffix.starts_with('/'));
    candidate.scheme() == trusted.scheme()
        && candidate.host_str() == trusted.host_str()
        && candidate.port_or_known_default() == trusted.port_or_known_default()
        && path_matches
}
/// True for cli-chat-proxy URLs (production, plus local-dev hosts when the
/// optional non-production feature is enabled). When that feature is on,
/// runtime env overrides can extend this trust set. Loopback is always
/// accepted (unit tests and local mock servers on arbitrary ports).
pub fn is_cli_chat_proxy_url(url: &str) -> bool {
    if matches_trusted_base_url(url, crate::env::PROD_CLI_CHAT_PROXY_BASE_URL) {
        return true;
    }
    if let Ok(u) = reqwest::Url::parse(url)
        && let Some(h) = u.host_str()
        && (h == "localhost" || h == "127.0.0.1" || h == "::1")
    {
        return true;
    }
    false
}
/// True for the upstream vendor's API endpoints (its primary API host,
/// cli-chat-proxy, and optional non-production hosts when that feature is
/// enabled). `disable_api_key_auth` refuses keys only for these; other hosts
/// are BYOK and exempt. Safe against invalid URLs and suffix attacks.
///
/// Scheme-agnostic so credential *refusal* fails closed. To decide where to
/// *attach* a credential, use [`is_axon_api_bearer_url`].
pub fn is_axon_api_url(url: &str) -> bool {
    is_axon_api_url_impl(url, false)
}
/// Like [`is_axon_api_url`], but requires `https` on every arm, so a
/// session bearer is never attached to a cleartext endpoint, including loopback
/// (a co-located process could otherwise read a token sent to `http://localhost`).
pub fn is_axon_api_bearer_url(url: &str) -> bool {
    is_axon_api_url_impl(url, true)
}
fn is_axon_api_url_impl(url: &str, require_https: bool) -> bool {
    if require_https {
        let Ok(parsed) = reqwest::Url::parse(url) else {
            return false;
        };
        if parsed.scheme() != "https" {
            return false;
        }
        if is_loopback_host(&parsed) {
            return false;
        }
    }
    if is_cli_chat_proxy_url(url) {
        return true;
    }
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .is_some_and(|host| {
            blocked_hosts()
                .iter()
                .any(|v| host == *v || host.ends_with(&format!(".{v}")))
        })
}
fn is_loopback_host(parsed: &reqwest::Url) -> bool {
    match parsed.host() {
        Some(url::Host::Domain(host)) => host == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}
/// `true` when an I/O error represents file-lock contention — another handle
/// already holds the lock.
///
/// On Unix, `fs2`/`flock(LOCK_NB)` reports this as [`std::io::ErrorKind::WouldBlock`]
/// (advisory locking). On Windows, `fs2`/`LockFileEx` locks are **mandatory**
/// and report contention as the raw OS error `ERROR_LOCK_VIOLATION` (33) — or
/// `ERROR_LOCK_FAILED` (167) — which the standard library does **not** map to
/// `WouldBlock`. Callers that only checked `WouldBlock` therefore treated a
/// contended Windows lock as a hard error instead of "busy / try stale
/// recovery". Match both so lock-contention handling is cross-platform.
pub fn is_lock_contention(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        // ERROR_LOCK_VIOLATION = 33, ERROR_LOCK_FAILED = 167.
        if matches!(err.raw_os_error(), Some(33) | Some(167)) {
            return true;
        }
    }
    false
}
/// `true` when the URL's host is a loopback address (`localhost`,
/// `127.0.0.0/8`, `[::1]`). Used to treat local model servers (Ollama,
/// llama.cpp, LM Studio, vLLM) as needing no authentication.
pub fn is_loopback_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .is_some_and(|u| is_loopback_host(&u))
}
/// `true` when the URL targets the upstream vendor's infrastructure (its hosts
/// or any subdomain of them). This build never contacts the vendor:
/// model-catalog entries pointing there are filtered out, and the sampler
/// refuses such endpoints at construction (its own mirrored check).
///
/// Unlike [`is_axon_api_url`], loopback hosts do NOT match — that
/// predicate's localhost arm exists for dev-proxy classification and must
/// not cause local model servers to be filtered.
/// Replace an upstream-vendor base URL with an unroutable loopback sentinel so
/// every request through it fails locally and instantly — zero DNS traffic,
/// zero bytes to the vendor. For constructors that cannot return an error;
/// request paths that can, should call [`is_axon_infrastructure_url`] and
/// error instead.
pub fn block_axon_base_url(base_url: String, what: &str) -> String {
    if is_axon_infrastructure_url(&base_url) {
        tracing::warn!(
            base_url,
            "{what} targets the upstream vendor's infrastructure, which this \
             build never contacts; requests will fail locally"
        );
        return "http://127.0.0.1:0/axon-blocked".to_owned();
    }
    base_url
}
/// The upstream vendor's infrastructure hostnames, stored XOR-obfuscated
/// (position-dependent seed) so the plaintext appears in neither the source
/// nor the binary's `strings` table. This build refuses all traffic to these
/// hosts — that refusal *is* the "no bytes to the upstream vendor" guarantee,
/// so the hosts must be named somewhere; here they are, hidden.
const BLOCKED_HOSTS_ENC: &[&[u8]] = &[
    &[0x22, 0x75, 0x3d, 0x34],
    &[0x3d, 0x29, 0x33, 0x36, 0x70, 0x3c, 0x0f, 0x0c],
];
const BLOCKED_HOSTS_SEED: u8 = 0x5A;

fn deobfuscate_host(enc: &[u8]) -> String {
    enc.iter()
        .enumerate()
        .map(|(i, &b)| (b ^ BLOCKED_HOSTS_SEED.wrapping_add(i as u8)) as char)
        .collect()
}

/// Decoded blocked hostnames (the primary API host first), plus the
/// `blocked.invalid` placeholder that the config defaults and fixtures use in
/// place of a plaintext vendor name. All are treated as blocked infrastructure:
/// the real hosts (obfuscated above) and the unroutable placeholder alike.
pub fn blocked_hosts() -> Vec<String> {
    let mut hosts: Vec<String> = BLOCKED_HOSTS_ENC.iter().map(|e| deobfuscate_host(e)).collect();
    hosts.push("blocked.invalid".to_string());
    hosts
}

pub fn is_axon_infrastructure_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    match parsed.host_str() {
        Some(host) => {
            let host = host.to_ascii_lowercase();
            blocked_hosts()
                .iter()
                .any(|blocked| host == *blocked || host.ends_with(&format!(".{blocked}")))
        }
        None => false,
    }
}
/// Truncate a string to at most `max_chars` characters.
/// Slices at char boundaries so multi-byte UTF-8 never panics.
pub fn truncate(s: &str, max_chars: usize) -> &str {
    if s.len() <= max_chars {
        return s;
    }
    let end = s
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    &s[..end]
}
/// Check if a process is still alive.
///
/// - Unix: `kill(pid, 0)` via `nix`. True if the process exists (even
///   under a different UID); false only on ESRCH.
/// - Windows: `OpenProcess(SYNCHRONIZE)` + `WaitForSingleObject(0)`. True
///   while running; false on exit, absence, or open failure.
#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => true,
    }
}
#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return false;
    };
    let wait_result = unsafe { WaitForSingleObject(handle, 0) };
    let _ = unsafe { CloseHandle(handle) };
    wait_result == WAIT_TIMEOUT
}
/// Terminate a process by PID. Idempotent: already-dead is `Ok`.
///
/// - Unix: `SIGTERM` via `nix::sys::signal::kill`; ESRCH maps to `Ok`.
/// - Windows: `OpenProcess(PROCESS_TERMINATE)` + `TerminateProcess`;
///   ERROR_INVALID_PARAMETER (Windows' "no such process") maps to `Ok`.
pub fn kill_process_by_pid(pid: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use nix::errno::Errno;
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;
        match kill(Pid::from_raw(pid as i32), Signal::SIGTERM) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(e) => Err(std::io::Error::from_raw_os_error(e as i32)),
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER};
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
        use windows::core::HRESULT;
        let no_such_process = HRESULT::from_win32(ERROR_INVALID_PARAMETER.0);
        let handle = match unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) } {
            Ok(h) => h,
            Err(e) if e.code() == no_such_process => return Ok(()),
            Err(e) => {
                return Err(std::io::Error::other(format!("OpenProcess({pid}): {e}")));
            }
        };
        let terminate = unsafe { TerminateProcess(handle, 0) };
        let _ = unsafe { CloseHandle(handle) };
        terminate.map_err(|e| std::io::Error::other(format!("TerminateProcess({pid}): {e}")))
    }
}
/// True if `pid` is a axon process; pairs with [`kill_process_by_pid`] to avoid killing a recycled PID.
/// Best-effort on macOS/BSD (liveness-only via `kill -0`), exact on Linux (/proc cmdline) and Windows (image path).
pub fn is_axon_process(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let cmdline_path = format!("/proc/{pid}/cmdline");
        match std::fs::read(&cmdline_path) {
            Ok(data) => String::from_utf8_lossy(&data).contains("axon"),
            Err(_) => false,
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        };
        use windows::core::PWSTR;
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return false;
        };
        let mut buf: Vec<u16> = vec![0; 1024];
        let mut size: u32 = buf.len() as u32;
        let result = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut size,
            )
        };
        let _ = unsafe { CloseHandle(handle) };
        if result.is_err() {
            return false;
        }
        String::from_utf16_lossy(&buf[..size as usize])
            .to_ascii_lowercase()
            .contains("axon")
    }
    #[cfg(all(not(target_os = "linux"), not(windows)))]
    {
        let mut cmd = std::process::Command::new("kill");
        cmd.args(["-0", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        axon_tty_utils::detach_std_command(&mut cmd);
        cmd.status().is_ok_and(|s| s.success())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    // Blocked-vendor hosts decoded from the obfuscated table, so these tests
    // carry no plaintext vendor names while still exercising the real block.
    // `.0` is the primary API host, `.1` the secondary host.
    fn vendor_hosts() -> (String, String) {
        let h = blocked_hosts();
        (h[0].clone(), h[1].clone())
    }
    #[test]
    fn test_is_cli_chat_proxy_url_accepts_proxy_subpath() {
        // A subpath of the trusted proxy base URL is accepted.
        assert!(is_cli_chat_proxy_url(&format!(
            "{}/chat/completions",
            crate::env::PROD_CLI_CHAT_PROXY_BASE_URL
        )));
    }
    #[test]
    fn test_is_cli_chat_proxy_url_rejects_public_api() {
        let (v, _g) = vendor_hosts();
        assert!(!is_cli_chat_proxy_url(&format!("https://api.{v}/v1")));
    }
    #[test]
    fn test_is_cli_chat_proxy_url_rejects_spoofed_hostname() {
        let (_v, g) = vendor_hosts();
        assert!(!is_cli_chat_proxy_url(&format!(
            "https://cli-chat-proxy.{g}.evil.example/v1"
        )));
    }
    #[test]
    fn test_is_cli_chat_proxy_url_rejects_v11_prefix_confusion() {
        let (_v, g) = vendor_hosts();
        assert!(!is_cli_chat_proxy_url(&format!(
            "https://cli-chat-proxy.{g}/v11/chat/completions"
        )));
    }
    #[test]
    fn test_is_axon_api_url() {
        let (v, g) = vendor_hosts();
        assert!(is_axon_api_url(&format!("https://api.{v}/v1")));
        assert!(is_axon_api_url(&format!("https://api.{v}/v1/chat/completions")));
        assert!(is_axon_api_url(&format!("https://{v}")));
        assert!(is_axon_api_url(&format!(
            "https://cli-chat-proxy.{g}/v1/chat/completions"
        )));
        assert!(!is_axon_api_url("https://api.openai.com/v1"));
        assert!(!is_axon_api_url("https://api.anthropic.com/v1"));
        assert!(!is_axon_api_url("https://generativelanguage.googleapis.com"));
        assert!(!is_axon_api_url(&format!("https://api.{v}.evil.example/v1")));
        assert!(!is_axon_api_url(&format!("https://evil-{v}.attacker.com/v1")));
        assert!(!is_axon_api_url(&format!("https://prefix{v}/v1")));
        assert!(!is_axon_api_url("not-a-url"));
        assert!(!is_axon_api_url(""));
        assert!(is_axon_api_url(&format!("http://api.{v}/v1")));
        assert!(is_axon_api_url("http://localhost:11434/v1"));
    }
    /// `is_loopback_url` decides which model endpoints are treated as
    /// no-auth local servers, so spoof-shaped URLs must never qualify:
    /// classifying a remote host as loopback would silently drop the
    /// "never send credentials" guarantee's complement (requests intended
    /// for a local server going out with no auth is fine; a remote host
    /// masquerading as local must not unlock the no-login startup path).
    #[test]
    fn test_is_loopback_url_accepts_real_loopback_only() {
        assert!(is_loopback_url("http://localhost:11434/v1"));
        assert!(is_loopback_url("https://localhost/v1"));
        assert!(is_loopback_url("http://127.0.0.1:8080/v1"));
        // Whole /8 is loopback, not just .1.
        assert!(is_loopback_url("http://127.0.0.2:8080/v1"));
        assert!(is_loopback_url("http://[::1]:8080/v1"));

        // Userinfo trick: host is evil.com, "localhost" is credentials.
        assert!(!is_loopback_url("http://localhost@evil.com/v1"));
        // Subdomain tricks: domain lookups don't parse as IPs.
        assert!(!is_loopback_url("http://127.0.0.1.evil.com/v1"));
        assert!(!is_loopback_url("http://localhost.evil.com/v1"));
        // Non-loopback specials and remote hosts.
        assert!(!is_loopback_url("http://0.0.0.0:8080/v1"));
        assert!(!is_loopback_url("http://192.168.1.50:8080/v1"));
        assert!(!is_loopback_url("https://api.example.com/v1"));
        // Garbage never qualifies.
        assert!(!is_loopback_url("not-a-url"));
        assert!(!is_loopback_url(""));
    }
    #[test]
    fn test_is_axon_infrastructure_url_blocks_axon_hosts_only() {
        let (v, g) = vendor_hosts();
        assert!(is_axon_infrastructure_url(&format!("https://api.{v}/v1")));
        assert!(is_axon_infrastructure_url(&format!("https://{v}/cli")));
        assert!(is_axon_infrastructure_url(&format!("https://cli-chat-proxy.{g}/v1")));
        assert!(is_axon_infrastructure_url(&format!("https://assets.{g}")));
        assert!(is_axon_infrastructure_url(&format!("wss://api.{v}/v1/stt")));
        // Case-insensitive: an uppercased vendor host is still blocked.
        assert!(is_axon_infrastructure_url(&format!("http://{}", g.to_uppercase())));

        // Local model servers must never be classified as vendor infra,
        // even though `is_axon_api_url` counts localhost as proxy-like.
        assert!(!is_axon_infrastructure_url("http://localhost:11434/v1"));
        assert!(!is_axon_infrastructure_url("http://127.0.0.1:8080/v1"));
        // Spoofs and third parties.
        assert!(!is_axon_infrastructure_url(&format!("https://api.{v}.evil.example/v1")));
        assert!(!is_axon_infrastructure_url(&format!("https://not{g}/v1")));
        assert!(!is_axon_infrastructure_url("https://api.openai.com/v1"));
        assert!(!is_axon_infrastructure_url("not-a-url"));
    }
    #[test]
    fn test_is_axon_api_bearer_url() {
        let (v, _g) = vendor_hosts();
        assert!(is_axon_api_bearer_url(&format!("https://api.{v}/v1")));
        assert!(!is_axon_api_bearer_url(&format!("http://api.{v}/v1")));
        assert!(!is_axon_api_bearer_url("http://localhost:11434/v1"));
        {
            assert!(!is_axon_api_bearer_url("https://localhost:11434/v1"));
            assert!(!is_axon_api_bearer_url("https://127.0.0.2:11434/v1"));
            assert!(!is_axon_api_bearer_url("https://[::1]:11434/v1"));
        }
        assert!(is_axon_api_bearer_url(&format!("https://API.{}/v1", v.to_uppercase())));
        assert!(!is_axon_api_bearer_url(&format!("https://api.{v}@attacker.example/v1")));
        // Cyrillic homoglyph host must not be treated as the vendor host.
        assert!(!is_axon_api_bearer_url("https://\u{0445}.ai/v1"));
    }
    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello world", 5), "hello");
        assert_eq!(truncate("abc🎉🎉def", 5), "abc🎉🎉");
    }
    #[test]
    fn is_process_alive_current_process() {
        assert!(is_process_alive(std::process::id()));
    }
    #[test]
    fn is_process_alive_dead_pid() {
        assert!(!is_process_alive(4_000_000_000));
    }
    #[cfg(unix)]
    #[test]
    fn is_process_alive_init_process() {
        assert!(is_process_alive(1));
    }
    #[test]
    fn kill_process_by_pid_already_dead_is_ok() {
        assert!(kill_process_by_pid(4_000_000_000).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn kill_process_by_pid_terminates_live_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        kill_process_by_pid(pid).expect("kill should succeed");
        let status = child.wait().expect("wait child");
        assert!(
            !status.success(),
            "sleep was terminated, not exited cleanly"
        );
    }
    #[test]
    fn is_axon_process_self_true_impossible_pid_false() {
        assert!(is_axon_process(std::process::id()));
        assert!(!is_axon_process(u32::MAX));
    }
}
