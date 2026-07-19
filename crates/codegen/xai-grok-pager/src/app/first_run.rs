//! First-run local-model setup wizard.
//!
//! This build never contacts xAI, so a fresh install with no `[model.*]`
//! configured would otherwise dead-end at an un-completable login screen. When
//! we detect that state at startup (before the TUI's terminal setup), we run a
//! small plain-terminal wizard: probe for a running local model server, let the
//! user pick a model (or enter an endpoint manually), and write it to
//! `~/.grok/config.toml`. The caller then reloads config and the TUI starts
//! straight into a session — no login, no race.

use std::io::{IsTerminal, Write};

use xai_grok_shell::local_setup::{
    LocalModelServer, config_id_for_model, probe_local_model_servers, write_local_model_config,
};

/// What the first-run wizard decided.
pub(crate) enum Outcome {
    /// Wrote a model to config; the caller should reload config and continue.
    Configured,
    /// Nothing to do (gate didn't fire); the caller continues unchanged.
    Skip,
    /// The user chose to quit setup without configuring a model; the caller
    /// should exit cleanly rather than fall through to the dead login screen.
    Quit,
}

/// Run the first-run wizard if it's warranted.
///
/// It runs only when: stdin/stdout are a real terminal, no `XAI_API_KEY` is
/// set, and the resolved catalog has no user-visible model (i.e. the app would
/// otherwise show the dead login screen). Any of those false → [`Outcome::Skip`].
pub(crate) async fn maybe_run(raw_config: &toml::Value) -> Outcome {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Outcome::Skip;
    }
    if xai_grok_shell::agent::auth_method::has_xai_api_key_env() {
        return Outcome::Skip;
    }
    if !needs_local_model_setup(raw_config) {
        return Outcome::Skip;
    }
    run_wizard().await
}

/// True only when the config parses AND its resolved catalog has no
/// user-visible model — the exact state that would dead-end at the removed
/// login screen. An unparseable config returns false (don't run the wizard on
/// a broken config); xAI-hosted defaults are hidden in this fork, so a fresh
/// install resolves to zero visible models.
fn needs_local_model_setup(raw_config: &toml::Value) -> bool {
    match xai_grok_shell::agent::config::Config::new_from_toml_cfg(raw_config) {
        Ok(cfg) => {
            let models = xai_grok_shell::agent::config::resolve_model_list(&cfg, None);
            !models.values().any(|m| !m.info.hidden)
        }
        Err(_) => false,
    }
}

/// A concrete "pick this to use it" choice: one model on one server.
struct Choice {
    label: &'static str,
    base_url: String,
    model: String,
}

fn flatten(servers: &[LocalModelServer]) -> Vec<Choice> {
    servers
        .iter()
        .flat_map(|s| {
            s.models.iter().map(move |m| Choice {
                label: s.label,
                base_url: s.base_url.clone(),
                model: m.clone(),
            })
        })
        .collect()
}

/// Read one trimmed line from stdin without blocking the async runtime.
async fn read_line() -> String {
    tokio::task::spawn_blocking(|| {
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
        s.trim().to_string()
    })
    .await
    .unwrap_or_default()
}

fn banner() {
    println!();
    println!("  ── Local model setup ─────────────────────────────────────────");
    println!("  This build talks only to local or third-party models — there is");
    println!("  no xAI login. No model is configured yet, so let's add one.");
    println!();
}

/// Message shown when the user quits setup without configuring a model.
fn print_quit_hint() {
    println!();
    println!("  No model configured. Add a [model.*] entry to ~/.grok/config.toml");
    println!("  (see the custom-models guide), then run grok again.");
    println!();
}

async fn run_wizard() -> Outcome {
    banner();
    let config_path = xai_grok_shell::util::grok_home::grok_home().join("config.toml");
    loop {
        print!("  Scanning localhost for a model server… ");
        let _ = std::io::stdout().flush();
        let servers = probe_local_model_servers().await;

        if servers.is_empty() {
            println!("none found.");
            println!();
            println!("  Start one — e.g. `ollama serve`, LM Studio, llama.cpp, or vLLM —");
            println!("  then rescan. Options:");
            println!("    [r] rescan   [m] enter an endpoint manually   [q] quit");
            print!("  > ");
            let _ = std::io::stdout().flush();
            match read_line().await.to_lowercase().as_str() {
                "r" | "" => continue,
                "m" => {
                    if manual_entry(&config_path).await {
                        return Outcome::Configured;
                    }
                    continue;
                }
                _ => {
                    print_quit_hint();
                    return Outcome::Quit;
                }
            }
        }

        let choices = flatten(&servers);
        println!("done.");
        println!();
        println!("  Detected local models:");
        for (i, c) in choices.iter().enumerate() {
            println!("    [{}] {}  ({})", i + 1, c.model, c.label);
        }
        println!("    [r] rescan   [m] manual endpoint   [q] quit");
        print!("  Pick a number > ");
        let _ = std::io::stdout().flush();

        let answer = read_line().await.to_lowercase();
        match answer.as_str() {
            "r" => continue,
            "m" => {
                if manual_entry(&config_path).await {
                    return Outcome::Configured;
                }
                continue;
            }
            "q" => {
                print_quit_hint();
                return Outcome::Quit;
            }
            _ => {
                if let Ok(n) = answer.parse::<usize>()
                    && n >= 1
                    && n <= choices.len()
                {
                    let c = &choices[n - 1];
                    // Detected servers are all loopback → auto no-auth.
                    if write_and_report(&config_path, &c.base_url, &c.model, false) {
                        return Outcome::Configured;
                    }
                    // Write failed (error already printed) — back to the menu.
                    continue;
                }
                println!("  Not a valid choice.");
                continue;
            }
        }
    }
}

/// Manual endpoint entry. Returns true if a model was written.
async fn manual_entry(config_path: &std::path::Path) -> bool {
    println!();
    print!("  OpenAI-compatible base URL (e.g. http://localhost:8080/v1) > ");
    let _ = std::io::stdout().flush();
    let base_url = read_line().await;
    if base_url.is_empty() {
        return false;
    }
    print!("  Model id the server expects > ");
    let _ = std::io::stdout().flush();
    let model = read_line().await;
    if model.is_empty() {
        return false;
    }
    // A non-loopback endpoint needs an explicit no-auth marker (loopback is
    // auto-detected). We assume a keyless local/LAN server here; a keyed
    // provider can be configured by hand later.
    let no_auth = !xai_grok_shell::util::is_loopback_url(&base_url);
    write_and_report(config_path, &base_url, &model, no_auth)
}

fn write_and_report(
    config_path: &std::path::Path,
    base_url: &str,
    model: &str,
    no_auth: bool,
) -> bool {
    let id = config_id_for_model(model);
    match write_local_model_config(config_path, &id, base_url, model, no_auth) {
        Ok(()) => {
            println!();
            println!("  ✓ Configured “{model}” at {base_url} (default model).");
            println!("    Starting…");
            println!();
            true
        }
        Err(e) => {
            eprintln!("  Failed to write {}: {e}", config_path.display());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_setup_true_for_fresh_install_false_with_local_model() {
        // Fresh install: no [model.*] → only hidden xAI defaults → needs setup.
        let empty: toml::Value = toml::from_str("").unwrap();
        assert!(needs_local_model_setup(&empty));

        // A configured local model is visible → no setup needed.
        let with_local: toml::Value = toml::from_str(
            r#"
            [model.local]
            model = "m"
            base_url = "http://localhost:11434/v1"
            context_window = 8192
            "#,
        )
        .unwrap();
        assert!(!needs_local_model_setup(&with_local));
    }

    #[test]
    fn flatten_expands_servers_to_model_choices() {
        let servers = vec![
            LocalModelServer {
                label: "Ollama",
                base_url: "http://localhost:11434/v1".into(),
                models: vec!["a".into(), "b".into()],
            },
            LocalModelServer {
                label: "vLLM",
                base_url: "http://localhost:8000/v1".into(),
                models: vec!["c".into()],
            },
        ];
        let choices = flatten(&servers);
        assert_eq!(choices.len(), 3);
        assert_eq!(choices[0].model, "a");
        assert_eq!(choices[0].label, "Ollama");
        assert_eq!(choices[2].model, "c");
        assert_eq!(choices[2].base_url, "http://localhost:8000/v1");
    }
}
