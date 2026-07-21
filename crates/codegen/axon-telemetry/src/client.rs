//! Core telemetry tracking — stubbed out.
//!
//! This build has all first-party analytics egress removed: the Mixpanel
//! client, the Axon product-events endpoint, and the people-profile sync are
//! gone. [`track`] is a no-op. The [`TelemetryMode`] plumbing is kept so
//! configuration still parses and mode queries keep their semantics, but no
//! event ever leaves the machine.

use std::sync::{Mutex, OnceLock};

use chrono::{Local, SecondsFormat};

use crate::config::{TelemetryConfig, TelemetryMode};
use crate::http::OriginClientInfo;

/// Event property map shared by all telemetry modules.
pub type Metadata = serde_json::Map<String, serde_json::Value>;

/// Retained mode marker. Formerly held Mixpanel/product-events credentials
/// and an HTTP client; now only records the configured mode.
#[derive(Clone, Debug)]
pub struct TelemetryClient {
    mode: TelemetryMode,
}

static TELEMETRY_CLIENT: OnceLock<Mutex<Option<TelemetryClient>>> = OnceLock::new();

/// Returns `true` when telemetry mode is `Enabled`.
pub fn is_enabled() -> bool {
    TELEMETRY_CLIENT
        .get()
        .and_then(|m| m.lock().ok())
        .is_some_and(|g| g.as_ref().is_some_and(|c| c.mode.is_enabled()))
}

/// Returns `true` when telemetry mode is `Enabled` or `SessionMetrics`.
pub fn is_session_metrics_enabled() -> bool {
    TELEMETRY_CLIENT
        .get()
        .and_then(|m| m.lock().ok())
        .is_some_and(|g| g.as_ref().is_some_and(|c| c.mode.session_metrics_enabled()))
}

pub struct UserContext {
    pub country: String,
    pub language: String,
    pub timestamp: String,
}

impl UserContext {
    pub fn collect() -> Self {
        let default_language = whoami::Language::En(whoami::Country::Any);
        let lang = whoami::langs()
            .ok()
            .and_then(|mut langs| langs.next())
            .unwrap_or(default_language);
        Self {
            country: lang.country().to_string(),
            language: lang.to_string(),
            timestamp: Local::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        }
    }
}

/// Former product-events + Mixpanel emitter. Egress removed; does nothing.
pub async fn track(_event_name: &str, _request_id: &str, _ctx: &UserContext, _metadata: Metadata) {}

/// Initialize the (stub) telemetry client. Safe to call multiple times.
/// Only the mode is retained; every credential/config argument is ignored.
#[allow(clippy::too_many_arguments)]
pub fn init(
    _config: TelemetryConfig,
    mode: TelemetryMode,
    _user_id: Option<String>,
    _team_id: Option<String>,
    _deployment_key: Option<String>,
    _origin_client: Option<OriginClientInfo>,
    _shell_version: String,
    _subscription_tier: Option<String>,
    _http_client: reqwest::Client,
) {
    let lock = TELEMETRY_CLIENT.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().unwrap_or_else(|err| err.into_inner());
    *guard = if mode.is_disabled() {
        None
    } else {
        Some(TelemetryClient { mode })
    };
}

/// Re-initialize the (stub) telemetry client if it was not created at startup.
/// No-op when the client is already set.
#[allow(clippy::too_many_arguments)]
pub fn init_if_needed(
    _config: TelemetryConfig,
    mode: TelemetryMode,
    _user_id: Option<String>,
    _team_id: Option<String>,
    _deployment_key: Option<String>,
    _origin_client: Option<OriginClientInfo>,
    _shell_version: String,
    _subscription_tier: Option<String>,
    _http_client: reqwest::Client,
) {
    if mode.is_disabled() {
        return;
    }
    let lock = TELEMETRY_CLIENT.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().unwrap_or_else(|err| err.into_inner());
    if guard.is_none() {
        *guard = Some(TelemetryClient { mode });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mode gates must keep their semantics even with egress removed:
    /// SessionMetrics reports session-metrics-enabled but not fully enabled.
    #[test]
    fn mode_gates_survive_stubbing() {
        // Clear the global client even if an assert below panics.
        struct ClearClient;
        impl Drop for ClearClient {
            fn drop(&mut self) {
                let lock = TELEMETRY_CLIENT.get_or_init(|| Mutex::new(None));
                *lock.lock().unwrap_or_else(|err| err.into_inner()) = None;
            }
        }
        let _clear = ClearClient;

        init(
            TelemetryConfig::default(),
            TelemetryMode::SessionMetrics,
            Some("user-1".into()),
            None,
            None,
            None,
            "0.0.0-test".into(),
            None,
            reqwest::Client::new(),
        );
        assert!(is_session_metrics_enabled());
        assert!(!is_enabled(), "product analytics must stay off");
    }
}
