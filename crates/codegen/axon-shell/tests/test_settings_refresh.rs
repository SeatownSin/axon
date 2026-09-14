//! Integration test: MockInferenceServer `/v1/settings` endpoint and
//! remote settings settings refresh infrastructure.
//!
//! Tests the mock endpoint directly (no binary needed) and verifies
//! the `fetch_settings_blocking` client round-trips correctly with
//! runtime-mutated mock settings.
//!
//! Run locally:
//! ```bash
//! cargo test -p axon-shell --test test_settings_refresh
//! ```

use std::future::Future;

use axon_shell::util::config::RemoteSettings;
use axon_test_support::*;

async fn with_local_set<F, Fut>(f: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    tokio::task::LocalSet::new().run_until(f()).await;
}

/// Verify the mock `/v1/settings` endpoint returns 404 when no settings
/// are configured (the default). This preserves backward compatibility:
/// existing tests that never call `set_settings` see a 404. Note this says
/// nothing about the client — `fetch_settings_blocking` answers `None` without
/// asking, 404 or not (see
/// `test_fetch_settings_blocking_never_contacts_the_server`).
#[tokio::test]
async fn test_settings_endpoint_returns_404_when_unconfigured() {
    with_local_set(|| async {
        let server = MockInferenceServer::start()
            .await
            .expect("start mock server");

        let resp = reqwest::get(format!("{}/settings", server.url()))
            .await
            .expect("request failed");
        assert_eq!(resp.status(), 404);
    })
    .await;
}

/// Verify the mock `/v1/settings` endpoint returns configured settings
/// and that `set_settings` runtime mutation is reflected immediately.
#[tokio::test]
async fn test_settings_endpoint_returns_configured_settings() {
    with_local_set(|| async {
        let server = MockInferenceServer::start()
            .await
            .expect("start mock server");

        // Configure initial settings
        server.set_settings(RemoteSettings {
            tips: Some(vec!["tip_v1".into()]),
            leader_mode: Some(false),
            ..Default::default()
        });

        // Fetch and verify
        let resp = reqwest::get(format!("{}/settings", server.url()))
            .await
            .expect("request failed");
        assert_eq!(resp.status(), 200);
        let settings: RemoteSettings = resp.json().await.expect("parse failed");
        assert_eq!(settings.tips, Some(vec!["tip_v1".into()]));
        assert_eq!(settings.leader_mode, Some(false));
    })
    .await;
}

/// Verify that `set_settings` updates are visible to subsequent requests
/// (runtime mutation for multi-session test scenarios).
#[tokio::test]
async fn test_settings_endpoint_reflects_runtime_mutations() {
    with_local_set(|| async {
        let server = MockInferenceServer::start()
            .await
            .expect("start mock server");

        // Initial settings
        server.set_settings(RemoteSettings {
            tips: Some(vec!["tip_v1".into()]),
            ..Default::default()
        });
        let settings: RemoteSettings = reqwest::get(format!("{}/settings", server.url()))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(settings.tips, Some(vec!["tip_v1".into()]));

        // Mutate settings (simulating a remote feature flag change)
        server.set_settings(RemoteSettings {
            tips: Some(vec!["tip_v2".into()]),
            leader_mode: Some(true),
            ..Default::default()
        });

        // Subsequent request sees the updated values
        let settings: RemoteSettings = reqwest::get(format!("{}/settings", server.url()))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(settings.tips, Some(vec!["tip_v2".into()]));
        assert_eq!(settings.leader_mode, Some(true));
    })
    .await;
}

/// `fetch_settings_blocking` is a stub, and this pins the removal rather than
/// the round-trip it used to perform: the remote-settings pull was deleted in
/// `f1beac4` so this build never asks cli-chat-proxy for announcements, promos,
/// campaigns or feature flags, and every caller already reads `None` as "use
/// local defaults". So the contract worth defending is that the answer stays
/// `None` **while the server is serving settings**, and that no request leaves
/// the process at all.
///
/// What this replaces: a round-trip test that could never pass, because no code
/// path can return `Some`. Worse, its first assertion — "`None` when settings
/// are not configured" — passed *vacuously*: the stub returns before any I/O,
/// so it never consulted the mock's 404. A probe that cannot fail sat next to
/// one that cannot pass, which is why the failure read as a broken round-trip
/// instead of a deleted function.
#[tokio::test]
async fn test_fetch_settings_blocking_never_contacts_the_server() {
    with_local_set(|| async {
        let server = MockInferenceServer::start()
            .await
            .expect("start mock server");

        // The one arrangement under which a *live* client would return Some.
        server.set_settings(RemoteSettings {
            tips: Some(vec!["fetched_tip".into()]),
            ..Default::default()
        });
        assert_eq!(server.request_count(), 0, "nothing has been asked yet");

        let auth = axon_shell::auth::AxonAuth {
            key: "test-key".into(),
            ..Default::default()
        };
        let result = tokio::task::spawn_blocking({
            let url = server.url().to_string();
            let auth = auth.clone();
            move || axon_shell::remote::fetch_settings_blocking(&url, &auth, None)
        })
        .await
        .unwrap();

        assert!(
            result.is_none(),
            "the remote-settings pull is removed; a Some here means it came back \
             (the settings struct is ~130 fields, so it is not dumped)"
        );
        assert_eq!(
            server.request_count(),
            0,
            "removed means no request at all, not a request whose answer is discarded: {:?}",
            server.requests()
        );
    })
    .await;
}

/// Verify the `doom_loop_recovery` settings object survives the
/// `/v1/settings` round-trip, that its absence deserializes to `None` (old
/// servers), and that a partial object keeps its unset fields `None`.
#[tokio::test]
async fn test_doom_loop_recovery_settings_round_trip() {
    use axon_shell::util::config::DoomLoopRecoverySettings;

    with_local_set(|| async {
        let server = MockInferenceServer::start()
            .await
            .expect("start mock server");

        // Absent from the payload ⇒ None on the client.
        server.set_settings(RemoteSettings::default());
        let settings: RemoteSettings = reqwest::get(format!("{}/settings", server.url()))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(settings.doom_loop_recovery, None);

        server.set_settings(RemoteSettings {
            doom_loop_recovery: Some(DoomLoopRecoverySettings {
                enabled: Some(true),
                max_threshold: Some(16),
                max_retries: Some(1),
            }),
            ..Default::default()
        });
        let settings: RemoteSettings = reqwest::get(format!("{}/settings", server.url()))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let recovery = settings.doom_loop_recovery.expect("object round-trips");
        assert_eq!(recovery.enabled, Some(true));
        assert_eq!(recovery.max_threshold, Some(16));
        assert_eq!(recovery.max_retries, Some(1));

        // Partial object: only the set field comes through; the rest stay
        // None so the resolver falls through per-field.
        server.set_settings(RemoteSettings {
            doom_loop_recovery: Some(DoomLoopRecoverySettings {
                max_threshold: Some(32),
                ..Default::default()
            }),
            ..Default::default()
        });
        let settings: RemoteSettings = reqwest::get(format!("{}/settings", server.url()))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let recovery = settings.doom_loop_recovery.expect("object round-trips");
        assert_eq!(recovery.enabled, None);
        assert_eq!(recovery.max_threshold, Some(32));
        assert_eq!(recovery.max_retries, None);
    })
    .await;
}

/// Verify that the mock server's request log correctly tracks
/// GET /v1/settings requests for assertion in multi-session tests.
#[tokio::test]
async fn test_settings_requests_appear_in_request_log() {
    with_local_set(|| async {
        let server = MockInferenceServer::start()
            .await
            .expect("start mock server");
        server.set_settings(RemoteSettings::default());

        assert_eq!(server.request_count(), 0);

        // First request
        let _ = reqwest::get(format!("{}/settings", server.url()))
            .await
            .unwrap();
        let settings_reqs: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.method == "GET" && r.path.contains("/settings"))
            .collect();
        assert_eq!(settings_reqs.len(), 1, "Expected 1 settings request");

        // Second request (simulating /new refresh)
        let _ = reqwest::get(format!("{}/settings", server.url()))
            .await
            .unwrap();
        let settings_reqs: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.method == "GET" && r.path.contains("/settings"))
            .collect();
        assert_eq!(settings_reqs.len(), 2, "Expected 2 settings requests");
    })
    .await;
}
