//! First-run local-model setup.
//!
//! This build never contacts xAI, so a fresh install with no `[model.*]`
//! configured has nothing to talk to and the upstream login screen is a dead
//! end. These helpers let the TUI detect a running local model server
//! (Ollama, LM Studio, llama.cpp, vLLM) and write a `[model.<id>]` entry so a
//! session can start with no login. The pager drives the UI; this module is
//! the reusable detect + write logic.

use std::path::Path;

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

/// Well-known localhost model servers, in preference order. Each speaks the
/// OpenAI-compatible `GET /v1/models` API — Ollama included, via its `/v1`
/// compatibility layer.
const PROBE_TARGETS: &[(&str, &str)] = &[
    ("Ollama", "http://localhost:11434"),
    ("LM Studio", "http://localhost:1234"),
    ("llama.cpp", "http://localhost:8080"),
    ("vLLM", "http://localhost:8000"),
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

/// Probe every well-known localhost server concurrently and return those that
/// respond with at least one model. Best-effort and fast (short per-probe
/// timeout); a server that is down simply doesn't appear in the result.
pub async fn probe_local_model_servers() -> Vec<LocalModelServer> {
    let client = crate::http::shared_client();
    let probes = PROBE_TARGETS
        .iter()
        .map(|&(label, base)| probe_endpoint(&client, label, base));
    futures::future::join_all(probes)
        .await
        .into_iter()
        .flatten()
        .collect()
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
    entry["name"] = toml_edit::value(model);
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
}
