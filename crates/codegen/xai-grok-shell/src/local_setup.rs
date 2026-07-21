//! First-run local-model setup.
//!
//! This build never contacts xAI, so a fresh install with no `[model.*]`
//! configured has nothing to talk to and the upstream login screen is a dead
//! end. These helpers let the TUI detect a running local model server
//! (Ollama, LM Studio, llama.cpp, vLLM) and write a `[model.<id>]` entry so a
//! session can start with no login. The pager drives the UI; this module is
//! the reusable detect + write logic.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::time::Duration;

/// A detected local model server and the models it advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalModelServer {
    /// Human label, e.g. `"Ollama"`.
    pub label: &'static str,
    /// OpenAI-compatible base URL ending in `/v1` (ready to write as `base_url`).
    pub base_url: String,
    /// Model ids advertised by `GET /v1/models`.
    pub models: Vec<String>,
}

/// Well-known OpenAI-compatible model-server ports and the product that
/// conventionally serves on each. Probed on `localhost` and on every host of
/// the machine's private LAN subnet(s). Each speaks `GET /v1/models` — Ollama
/// included, via its `/v1` compatibility layer.
const PROBE_PORTS: &[(&str, u16)] = &[
    ("Ollama", 11434),
    ("LM Studio", 1234),
    ("llama.cpp", 8080),
    ("vLLM", 8000),
];

/// Extract model ids from an OpenAI `/v1/models` response body. Split out so
/// the parsing is unit-testable without a live server.
fn parse_model_ids(body: &serde_json::Value) -> Vec<String> {
    body.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Probe a single server's `GET {base}/v1/models`. Returns the server + its
/// model ids, or `None` if it's down, errors, or lists nothing. Split out so
/// the real HTTP + parse path is testable against a mock server.
async fn probe_endpoint(
    client: &reqwest::Client,
    label: &'static str,
    base: &str,
) -> Option<LocalModelServer> {
    let url = format!("{base}/v1/models");
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_millis(1500))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    let models = parse_model_ids(&body);
    if models.is_empty() {
        return None;
    }
    Some(LocalModelServer {
        label,
        base_url: format!("{base}/v1"),
        models,
    })
}

/// Probe localhost and the machine's private LAN subnet(s) for OpenAI-compatible
/// model servers, returning those that respond with at least one model.
/// Localhost is checked first (instant); the LAN sweep TCP-pre-checks each
/// `host:port` and only issues an HTTP request to ports that accept a
/// connection, so a full /24 finishes in a couple of seconds. Best-effort:
/// unreachable hosts simply don't appear.
pub async fn probe_local_model_servers() -> Vec<LocalModelServer> {
    let client = crate::http::shared_client();

    // Local candidates: the well-known ports on IPv4 loopback, PLUS every port
    // actually listening on this machine. LM Studio, llama.cpp & co. routinely
    // serve on a dynamic/non-standard port (e.g. LM Studio on 49152), so a
    // fixed list alone misses them. We also target `127.0.0.1`/`[::1]`
    // explicitly rather than `localhost` — on Windows `localhost` prefers IPv6
    // `::1`, and an unlistened `::1` silently drops (timeout) instead of
    // refusing, which would burn the probe budget before IPv4 is even tried.
    let mut candidates: Vec<(&'static str, String)> = PROBE_PORTS
        .iter()
        .map(|&(label, port)| (label, format!("http://127.0.0.1:{port}")))
        .collect();
    for (addr, port) in local_listening_ports() {
        let host = if addr.is_ipv6() { "[::1]" } else { "127.0.0.1" };
        candidates.push((label_for_port(port), format!("http://{host}:{port}")));
    }
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|(_, base)| seen.insert(base.clone()));

    let local = futures::future::join_all(candidates.iter().map(|(label, base)| {
        let client = &client;
        async move { probe_endpoint(client, label, base).await }
    }))
    .await;
    let mut found: Vec<LocalModelServer> = local.into_iter().flatten().collect();

    // LAN: sweep private subnets on the well-known ports, de-duplicating.
    for server in scan_lan(&client).await {
        if !found.iter().any(|f| f.base_url == server.base_url) {
            found.push(server);
        }
    }
    found
}

/// Product conventionally serving on a well-known port; `"Local server"` for
/// anything discovered on a dynamic/non-standard port.
fn label_for_port(port: u16) -> &'static str {
    match port {
        11434 => "Ollama",
        1234 => "LM Studio",
        8080 => "llama.cpp",
        8000 => "vLLM",
        _ => "Local server",
    }
}

/// TCP `(addr, port)` pairs in LISTEN state on this machine, bound to loopback
/// or an all-interfaces address (so reachable via localhost). Privileged ports
/// (<1024 — ssh, SMB, RPC) are skipped: model servers never run there and we
/// shouldn't poke system services. Empty if the socket table can't be read.
fn local_listening_ports() -> Vec<(IpAddr, u16)> {
    use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState};
    let sockets = match netstat2::get_sockets_info(
        AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
        ProtocolFlags::TCP,
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    sockets
        .into_iter()
        .filter_map(|si| match si.protocol_socket_info {
            ProtocolSocketInfo::Tcp(tcp)
                if tcp.state == TcpState::Listen
                    && tcp.local_port >= 1024
                    && is_local_reachable(tcp.local_addr) =>
            {
                Some((tcp.local_addr, tcp.local_port))
            }
            _ => None,
        })
        .collect()
}

/// True when a listener bound to `addr` is reachable over localhost: a loopback
/// address, or an all-interfaces bind (`0.0.0.0` / `::`).
fn is_local_reachable(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_unspecified(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

/// Cap on host:port probes a single LAN sweep will issue, so an unusually large
/// or misconfigured subnet can't turn setup into a long scan.
const MAX_LAN_PROBES: usize = 4096;
/// Concurrent in-flight probes during the LAN sweep.
const LAN_CONCURRENCY: usize = 128;
/// Per-host TCP connect budget. A refused port returns instantly, so this only
/// bounds hosts that silently drop (filtered/absent).
const LAN_CONNECT_TIMEOUT: Duration = Duration::from_millis(400);

/// Sweep the machine's private IPv4 /24 subnet(s) for model servers. Returns
/// empty when the host has no private LAN address (e.g. loopback-only). Only
/// private (RFC1918) ranges are ever touched — never a routable/public host.
async fn scan_lan(client: &reqwest::Client) -> Vec<LocalModelServer> {
    use futures::stream::StreamExt;

    let own = local_private_v4_ips();
    let hosts = candidate_hosts(&own, MAX_LAN_PROBES / PROBE_PORTS.len());
    if hosts.is_empty() {
        return Vec::new();
    }

    let targets: Vec<(Ipv4Addr, &'static str, u16)> = hosts
        .iter()
        .flat_map(|&host| PROBE_PORTS.iter().map(move |&(label, port)| (host, label, port)))
        .collect();

    futures::stream::iter(targets)
        .map(|(host, label, port)| {
            let client = client.clone();
            async move {
                if !tcp_open(host, port, LAN_CONNECT_TIMEOUT).await {
                    return None;
                }
                probe_endpoint(&client, label, &format!("http://{host}:{port}")).await
            }
        })
        .buffer_unordered(LAN_CONCURRENCY)
        .filter_map(|r| async move { r })
        .collect()
        .await
}

/// True if a TCP connection to `host:port` completes within `timeout`.
async fn tcp_open(host: Ipv4Addr, port: u16, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

/// The machine's own private (RFC1918) IPv4 addresses, one per interface.
fn local_private_v4_ips() -> Vec<Ipv4Addr> {
    if_addrs::get_if_addrs()
        .map(|ifaces| ifaces.iter().filter_map(|i| scannable_private_v4(i.ip())).collect())
        .unwrap_or_default()
}

/// Keep only private, non-loopback, non-link-local IPv4 addresses — the ones
/// whose /24 is worth sweeping. IPv6 and public addresses are dropped: the
/// sweep never reaches beyond the local wire, and never a routable range.
fn scannable_private_v4(ip: std::net::IpAddr) -> Option<Ipv4Addr> {
    match ip {
        std::net::IpAddr::V4(v4)
            if v4.is_private() && !v4.is_loopback() && !v4.is_link_local() =>
        {
            Some(v4)
        }
        _ => None,
    }
}

/// Expand each owned address's /24 into candidate host addresses (`.1`–`.254`),
/// de-duplicating shared subnets and excluding the machine's own addresses.
/// Capped at `max` hosts so the sweep stays bounded.
fn candidate_hosts(own: &[Ipv4Addr], max: usize) -> Vec<Ipv4Addr> {
    let own_set: std::collections::HashSet<Ipv4Addr> = own.iter().copied().collect();
    let mut seen_subnet = std::collections::HashSet::new();
    let mut hosts = Vec::new();
    for ip in own {
        let o = ip.octets();
        if !seen_subnet.insert([o[0], o[1], o[2]]) {
            continue;
        }
        for last in 1..=254u8 {
            let host = Ipv4Addr::new(o[0], o[1], o[2], last);
            if own_set.contains(&host) {
                continue;
            }
            hosts.push(host);
            if hosts.len() >= max {
                return hosts;
            }
        }
    }
    hosts
}

/// Derive a TOML-bare-key-friendly config section id from a model id, so the
/// `[model.<id>]` key and `[models].default` match without quoting surprises
/// (e.g. `llama3.1:8b` → `llama3-1-8b`; dots must not survive, or TOML would
/// read `[model.llama3.1]` as nested tables).
pub fn config_id_for_model(model: &str) -> String {
    let mapped: String = model
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = mapped.trim_matches('-');
    if trimmed.is_empty() {
        "local".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Human-friendly display name for a model id. Server model ids are often the
/// filesystem path to a weights file — llama.cpp names models by their `.gguf`
/// path — which reads terribly in a menu. Collapse those to the file stem
/// (last path component, known weights extension stripped). Non-path ids
/// (`qwen2.5-coder`, `gemma@q4_k_m`) are returned unchanged.
pub fn display_name_for_model(model: &str) -> String {
    if !model.contains('/') && !model.contains('\\') {
        return model.to_string();
    }
    let base = model.rsplit(['/', '\\']).next().unwrap_or(model);
    let stem = [".gguf", ".bin", ".safetensors", ".ggml"]
        .iter()
        .find_map(|ext| base.strip_suffix(ext))
        .unwrap_or(base);
    if stem.is_empty() {
        model.to_string()
    } else {
        stem.to_string()
    }
}

/// Write a `[model.<config_id>]` entry pointing at a local server and set it as
/// the default, preserving all existing config. `base_url` should be an
/// OpenAI-compatible endpoint ending in `/v1`.
///
/// Loopback URLs are auto-treated as no-auth, so `no_auth` may stay false for
/// them. For a non-loopback endpoint that needs no key (a LAN server), pass
/// `no_auth = true` so it, too, skips authentication.
pub fn write_local_model_config(
    config_path: &Path,
    config_id: &str,
    base_url: &str,
    model: &str,
    no_auth: bool,
) -> std::io::Result<()> {
    if let Some(parent) = config_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let existing = crate::util::config::read_to_string_or_empty(config_path)?;
    let mut doc = existing.parse::<toml_edit::DocumentMut>().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid TOML: {e}"))
    })?;

    let model_tbl = doc
        .entry("model")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "[model] is not a table")
        })?;
    // Implicit parent: emit only `[model.<id>]`, not a redundant empty `[model]`.
    model_tbl.set_implicit(true);
    let mut entry = toml_edit::Table::new();
    entry["model"] = toml_edit::value(model);
    entry["base_url"] = toml_edit::value(base_url);
    // Display name is prettified (a `.gguf` path collapses to its stem); the
    // `model` id above stays verbatim so the server gets exactly what it expects.
    entry["name"] = toml_edit::value(display_name_for_model(model).as_str());
    if no_auth {
        entry["no_auth"] = toml_edit::value(true);
    }
    model_tbl.insert(config_id, toml_edit::Item::Table(entry));

    let models_tbl = doc
        .entry("models")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "[models] is not a table")
        })?;
    models_tbl["default"] = toml_edit::value(config_id);

    crate::util::config::atomic_write_string(config_path, &doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_endpoint_detects_and_normalizes() {
        let mut server = mockito::Server::new_async().await;
        let m = server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"object":"list","data":[{"id":"llama3"},{"id":"qwen"}]}"#)
            .create_async()
            .await;
        let client = crate::http::shared_client();
        let got = probe_endpoint(&client, "Mock", &server.url())
            .await
            .expect("mock server must be detected");
        assert_eq!(got.models, vec!["llama3", "qwen"]);
        assert_eq!(got.base_url, format!("{}/v1", server.url()));
        m.assert_async().await;
    }

    #[tokio::test]
    async fn probe_endpoint_skips_empty_and_error() {
        let mut server = mockito::Server::new_async().await;
        let _empty = server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_body(r#"{"data":[]}"#)
            .create_async()
            .await;
        let client = crate::http::shared_client();
        assert!(probe_endpoint(&client, "Mock", &server.url()).await.is_none());
    }

    #[test]
    fn parse_model_ids_reads_openai_shape() {
        let body = serde_json::json!({
            "object": "list",
            "data": [{"id": "llama3.1:8b"}, {"id": "qwen2.5-coder"}, {"other": 1}]
        });
        assert_eq!(parse_model_ids(&body), vec!["llama3.1:8b", "qwen2.5-coder"]);
        assert!(parse_model_ids(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn config_id_is_toml_bare_key_safe() {
        assert_eq!(config_id_for_model("llama3.1:8b"), "llama3-1-8b");
        assert_eq!(config_id_for_model("Qwen2.5-Coder"), "qwen2-5-coder");
        assert_eq!(config_id_for_model("gpt-4o"), "gpt-4o");
        assert_eq!(config_id_for_model("///"), "local");
        // No dots survive — a dotted key would nest tables in TOML.
        assert!(!config_id_for_model("a.b.c").contains('.'));
    }

    #[test]
    fn write_creates_entry_and_default_preserving_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[cli]\nauto_update = false\n").unwrap();

        write_local_model_config(
            &path,
            "local-llama",
            "http://localhost:11434/v1",
            "llama3.1:8b",
            false,
        )
        .unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        // No redundant empty `[model]` header (implicit parent).
        assert!(
            !written.lines().any(|l| l.trim() == "[model]"),
            "unexpected empty [model] header:\n{written}"
        );
        let doc: toml_edit::DocumentMut = written.parse().unwrap();
        // Existing content preserved.
        assert_eq!(doc["cli"]["auto_update"].as_bool(), Some(false));
        // New model entry.
        assert_eq!(
            doc["model"]["local-llama"]["base_url"].as_str(),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(
            doc["model"]["local-llama"]["model"].as_str(),
            Some("llama3.1:8b")
        );
        // Default points at it.
        assert_eq!(doc["models"]["default"].as_str(), Some("local-llama"));

        // Re-parse through the real config loader to prove it round-trips.
        let toml: toml::Value = toml::from_str(&written).unwrap();
        assert!(toml.get("model").and_then(|m| m.get("local-llama")).is_some());
    }

    #[test]
    fn write_sets_no_auth_only_when_requested() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_local_model_config(&path, "lan", "http://192.168.1.9:8080/v1", "m", true).unwrap();
        let doc: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(doc["model"]["lan"]["no_auth"].as_bool(), Some(true));

        let path2 = dir.path().join("config2.toml");
        write_local_model_config(&path2, "lo", "http://localhost:11434/v1", "m", false).unwrap();
        let doc2: toml_edit::DocumentMut =
            std::fs::read_to_string(&path2).unwrap().parse().unwrap();
        assert!(doc2["model"]["lo"].get("no_auth").is_none());
    }
    #[test]
    fn write_reparses_via_model_override_parser() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_local_model_config(&path, "local", "http://127.0.0.1:1234/v1", "some-model", false)
            .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let cfg = crate::agent::config::Config::new_from_toml_cfg(&toml::from_str(&raw).unwrap())
            .expect("written config must parse");
        let models = crate::agent::config::resolve_model_list(&cfg, None);
        let entry = models.get("local").expect("local model resolves");
        assert_eq!(entry.info.base_url, "http://127.0.0.1:1234/v1");
        // Loopback → auto no-auth (no key, no login).
        assert!(entry.requires_no_auth());
    }

    #[test]
    fn scannable_private_v4_keeps_only_private_wire_addresses() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        let v4 = |a, b, c, d| IpAddr::V4(Ipv4Addr::new(a, b, c, d));
        // RFC1918 ranges are scannable.
        assert!(scannable_private_v4(v4(192, 168, 1, 10)).is_some());
        assert!(scannable_private_v4(v4(10, 0, 0, 5)).is_some());
        assert!(scannable_private_v4(v4(172, 16, 4, 9)).is_some());
        // Public, loopback, link-local, and any IPv6 are dropped — the sweep
        // never reaches a routable host.
        assert!(scannable_private_v4(v4(8, 8, 8, 8)).is_none());
        assert!(scannable_private_v4(v4(127, 0, 0, 1)).is_none());
        assert!(scannable_private_v4(v4(169, 254, 1, 1)).is_none());
        assert!(scannable_private_v4(IpAddr::V6(Ipv6Addr::LOCALHOST)).is_none());
    }

    #[test]
    fn candidate_hosts_dedupes_subnet_excludes_self_and_caps() {
        // Two addresses on the same /24 → 254 hosts minus the two own = 252.
        let same = [Ipv4Addr::new(192, 168, 1, 50), Ipv4Addr::new(192, 168, 1, 77)];
        let hosts = candidate_hosts(&same, MAX_LAN_PROBES);
        assert_eq!(hosts.len(), 252);
        assert!(!hosts.contains(&Ipv4Addr::new(192, 168, 1, 50)));
        assert!(!hosts.contains(&Ipv4Addr::new(192, 168, 1, 77)));
        assert!(hosts.contains(&Ipv4Addr::new(192, 168, 1, 1)));
        assert!(hosts.contains(&Ipv4Addr::new(192, 168, 1, 254)));

        // Cap is honored.
        assert_eq!(candidate_hosts(&same, 10).len(), 10);

        // Two distinct /24s are both swept (254 − 1 own each = 253 + 253).
        let two = [Ipv4Addr::new(192, 168, 1, 1), Ipv4Addr::new(10, 0, 0, 1)];
        assert_eq!(candidate_hosts(&two, MAX_LAN_PROBES).len(), 506);
    }

    #[test]
    fn label_for_port_maps_known_and_defaults() {
        assert_eq!(label_for_port(11434), "Ollama");
        assert_eq!(label_for_port(1234), "LM Studio");
        assert_eq!(label_for_port(8080), "llama.cpp");
        assert_eq!(label_for_port(8000), "vLLM");
        // A dynamic port (e.g. LM Studio's actual bind) is a generic local server.
        assert_eq!(label_for_port(49152), "Local server");
    }

    #[test]
    fn display_name_for_model_collapses_paths_keeps_plain_ids() {
        // Plain ids pass through untouched.
        assert_eq!(display_name_for_model("qwen2.5-coder"), "qwen2.5-coder");
        assert_eq!(display_name_for_model("gemma@q4_k_m"), "gemma@q4_k_m");
        // Windows `.gguf` path → file stem, extension stripped.
        assert_eq!(
            display_name_for_model(r"F:\AI\Models\GGUF\Gemma4-12B-Q4_K_M.gguf"),
            "Gemma4-12B-Q4_K_M"
        );
        // Unix path likewise.
        assert_eq!(
            display_name_for_model("/models/llama/Llama-3.1-8B-Instruct.gguf"),
            "Llama-3.1-8B-Instruct"
        );
        // A path without a known weights extension keeps its last component.
        assert_eq!(display_name_for_model("/srv/models/my-model"), "my-model");
    }

    #[test]
    fn is_local_reachable_accepts_loopback_and_unspecified_only() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        // Loopback and all-interfaces binds are reachable via localhost.
        assert!(is_local_reachable(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_local_reachable(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        assert!(is_local_reachable(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_local_reachable(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        // A LAN or public bind is not localhost-reachable.
        assert!(!is_local_reachable(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))));
        assert!(!is_local_reachable(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }
}
