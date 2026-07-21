//! Formerly Sentry crash/error reporting. Removed.
//!
//! [`init`] no longer reads `SENTRY_DSN` (env or build-time) and always
//! initializes a disabled, DSN-less client, so panics and errors are never
//! transmitted anywhere. The API surface is kept so binaries compile.

use std::sync::OnceLock;

use sentry::ClientInitGuard;
use sentry::ClientOptions;

// ─── Host integration ─────────────────────────────────────────────────────

/// Per-host config; retained for API compatibility. All reporting is
/// disabled regardless of these values.
pub struct Config {
    /// Sentry tag `client`, e.g. `"axon-pager"`. Unused.
    pub client: &'static str,
    pub client_version: &'static str,
    pub release: &'static str,
    /// Retained; reporting is disabled unconditionally.
    pub disabled: bool,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

// ─── Public API ────────────────────────────────────────────────────────────

/// Returns a no-op guard. No DSN is ever configured, so the Sentry client
/// is permanently disabled and nothing is captured or transmitted.
pub fn init(config: Config) -> ClientInitGuard {
    let _ = CONFIG.get_or_init(|| config);
    sentry::init(ClientOptions::default())
}

/// Formerly flushed in-flight events before exit. Nothing to flush now.
pub fn flush_on_shutdown() {}
