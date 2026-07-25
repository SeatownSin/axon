//! Plugin marketplace browse and index crate.
//!
//! Provides marketplace source configuration, plugin discovery (indexed +
//! filesystem fallback), and install integration with the existing
//! `InstallRegistry` pipeline.

pub mod catalog;
pub mod config;
pub mod error;
pub mod git;
pub mod index;
pub mod install_resolve;
pub mod installer;
pub mod matcher;
pub mod scanner;
pub mod types;

pub use config::{
    env_official_source, env_require_sha, load_extra_sources_from_settings,
    load_extra_sources_from_settings_in, load_official_source, load_require_sha, load_sources,
};
pub use error::MarketplaceError;
pub use scanner::scan_marketplace;
pub use types::*;

/// Display name given to the configured official marketplace source.
pub const OFFICIAL_SOURCE_NAME: &str = "Axon Official";

/// Whether `url` is the marketplace source configured as official, normalizing
/// case, a `www.` prefix, a trailing `/` or `.git`, and HTTPS/SSH forms before
/// comparing. `official` is the configured URL from
/// [`load_official_source`] — `None` (the default) means *nothing* is official,
/// so no source gets install privileges by default.
///
/// Comparison is by canonical URL only. Matching on the display name as well
/// would let any source call itself "Axon Official" and inherit the CTA's
/// install path.
pub fn is_official_source_url(official: Option<&str>, url: &str) -> bool {
    let Some(official) = official else {
        return false;
    };
    match canonical_github_owner_repo(official) {
        // Both GitHub URLs: compare canonical owner/repo so HTTPS/SSH/`.git`
        // spellings of the same repo agree.
        Some(official_repo) => canonical_github_owner_repo(url).as_deref() == Some(&official_repo),
        // Non-GitHub remote (self-hosted git, file path): exact match after the
        // same trailing-slash/`.git`/case normalisation.
        None => normalize_non_github(official) == normalize_non_github(url),
    }
}

/// Case/suffix normalisation for remotes [`canonical_github_owner_repo`] does
/// not understand, so a self-hosted marketplace can still be named official.
fn normalize_non_github(url: &str) -> String {
    let s = url.trim();
    let s = s.strip_suffix('/').unwrap_or(s);
    let s = s.strip_suffix(".git").unwrap_or(s);
    s.to_ascii_lowercase()
}

/// Normalized lowercase `owner/repo` from a GitHub URL (HTTPS/http/ssh/scp,
/// `www.`, trailing `.git`/`/`), or `None` if not a GitHub URL.
pub(crate) fn canonical_github_owner_repo(url: &str) -> Option<String> {
    let s = url.trim();
    let s = s.strip_suffix('/').unwrap_or(s);
    let s = s.strip_suffix(".git").unwrap_or(s);
    let lower = s.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .or_else(|| lower.strip_prefix("ssh://"))
        .unwrap_or(&lower);
    let rest = rest.strip_prefix("git@").unwrap_or(rest);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let owner_repo = rest
        .strip_prefix("github.com/")
        .or_else(|| rest.strip_prefix("github.com:"))?;
    if owner_repo.is_empty() {
        None
    } else {
        Some(owner_repo.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands in for whatever the user configures. Not a real repository --
    /// this build ships with no official source at all.
    const CONFIGURED: &str = "https://github.com/example-org/plugin-marketplace.git";

    fn official(url: &str) -> bool {
        is_official_source_url(Some(CONFIGURED), url)
    }

    /// The default: nothing configured, so nothing is official and no source
    /// inherits the CTA's install path.
    #[test]
    fn nothing_is_official_when_unconfigured() {
        assert!(!is_official_source_url(None, CONFIGURED));
        assert!(!is_official_source_url(
            None,
            "https://github.com/example-org/plugin-marketplace"
        ));
        assert!(!is_official_source_url(None, ""));
    }

    #[test]
    fn is_official_matches_canonical_https() {
        assert!(official(CONFIGURED));
        assert!(official(
            "https://github.com/example-org/plugin-marketplace"
        ));
    }

    #[test]
    fn is_official_matches_ssh_form() {
        assert!(official(
            "git@github.com:example-org/plugin-marketplace.git"
        ));
        assert!(official("git@github.com:example-org/plugin-marketplace"));
        assert!(official(
            "ssh://git@github.com/example-org/plugin-marketplace.git"
        ));
        assert!(official(
            "ssh://git@github.com/example-org/plugin-marketplace"
        ));
    }

    #[test]
    fn is_official_rejects_unrelated_urls() {
        assert!(!official(
            "https://github.com/anthropics/claude-plugins-official.git"
        ));
        assert!(!official(
            "https://github.com/example-org/some-other-repo.git"
        ));
        assert!(!official("https://github.com/other-org/plugin-marketplace"));
        assert!(!official(""));
    }

    #[test]
    fn is_official_matches_noncanonical_forms() {
        assert!(official(
            "https://GitHub.com/EXAMPLE-org/Plugin-Marketplace"
        ));
        assert!(official(
            "https://github.com/example-org/plugin-marketplace/"
        ));
        assert!(official(
            "https://github.com/example-org/plugin-marketplace.git/"
        ));
        assert!(official("http://github.com/example-org/plugin-marketplace"));
        assert!(official(
            "https://www.github.com/example-org/plugin-marketplace.git"
        ));
        assert!(official(
            "git@github.com:EXAMPLE-org/plugin-marketplace.git"
        ));
    }

    /// A self-hosted marketplace can be named official too -- neither side is
    /// a GitHub URL, so the comparison falls back to normalised equality.
    #[test]
    fn is_official_matches_non_github_remote() {
        let self_hosted = "https://git.example.test/plugins/marketplace.git";
        assert!(is_official_source_url(
            Some(self_hosted),
            "https://git.example.test/plugins/marketplace"
        ));
        assert!(is_official_source_url(Some(self_hosted), self_hosted));
        assert!(!is_official_source_url(
            Some(self_hosted),
            "https://git.example.test/plugins/other"
        ));
        // A GitHub URL never matches a non-GitHub official source.
        assert!(!is_official_source_url(Some(self_hosted), CONFIGURED));
    }
}
