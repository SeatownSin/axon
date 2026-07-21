//! Formerly the OpenTelemetry span-export layer to the cli-chat-proxy.
//!
//! The exporter has been removed: no spans are shipped to any first-party
//! backend. The config types and the layer constructor are kept so binary
//! wiring compiles, but [`build_otel_layer`] returns an inert layer and
//! [`shutdown_otel`] only flushes the *external* (customer-controlled) OTLP
//! stream, which remains available via the `external` module.
use std::sync::Arc;
use tracing_subscriber::registry::LookupSpan;
use axon_auth::AuthCredentialProvider;

/// Configuration for [`build_otel_layer`]. Retained for API compatibility;
/// its contents are no longer used since span export was removed.
pub struct OtelLayerConfig {
    /// Formerly the live credential source for export auth headers. Unused.
    pub credentials: Arc<dyn AuthCredentialProvider>,
    /// Formerly the `X-AXON-Token-Auth` header value. Unused.
    pub token_header_value: String,
    /// Formerly an extra traces access key. Unused.
    pub alpha_test_key: Option<String>,
    pub exporter: OtelExporterConfig,
}

/// Static identity of the client emitting telemetry. Unused since span
/// export was removed.
#[derive(Debug, Clone, Copy)]
pub struct OtelClientInfo {
    pub client_name: &'static str,
    pub client_version: &'static str,
    pub service_version: &'static str,
    pub app_entrypoint: &'static str,
}

/// OTLP trace-export transport settings. Retained so configuration
/// resolution keeps compiling; nothing reads it anymore.
#[derive(Debug, Default, Clone)]
pub struct OtelExporterConfig {
    pub traces_url: String,
    pub extra_headers: Vec<(String, String)>,
    pub export_interval: Option<std::time::Duration>,
    pub timeout: Option<std::time::Duration>,
    pub enabled: bool,
}

/// Returns an inert layer. Span export to the cli-chat-proxy has been
/// removed from this build; no tracer provider or exporter is created.
pub fn build_otel_layer<S>(
    _client: OtelClientInfo,
    _config: OtelLayerConfig,
) -> impl tracing_subscriber::layer::Layer<S>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    tracing_subscriber::layer::Identity::new()
}

/// Flush and shut down the external OTEL stream (customer-controlled
/// collector). The first-party span exporter no longer exists.
///
/// Safe to call multiple times.
pub fn shutdown_otel() {
    crate::external::shutdown();
}

/// RAII guard that calls [`shutdown_otel`] on drop.
pub struct OtelGuard;
impl Drop for OtelGuard {
    fn drop(&mut self) {
        shutdown_otel();
    }
}

/// Create an [`OtelGuard`] that flushes the external stream on drop.
pub fn otel_guard() -> OtelGuard {
    OtelGuard
}
