pub mod config;
pub mod axon_auth_credentials;
pub mod hooks;

// The foundation utilities live in `axon-shell-base` (upstream of this
// crate so they build in parallel). Re-exported at the original paths so
// existing `crate::util::…` / `axon_shell::util::…` users compile
// unchanged.
pub use axon_shell_base::util::*;

pub(crate) fn is_user_instruction_path(
    path: &std::path::Path,
    axon_home: &std::path::Path,
    vendor_homes: &[(std::path::PathBuf, bool)],
    workspace_root: Option<&std::path::Path>,
) -> bool {
    let parent = path.parent();
    let axon_rules = axon_home.join("rules");
    let is_exact_home_surface = parent
        .is_some_and(|parent| parent == axon_home || parent == axon_rules)
        || vendor_homes.iter().any(|(vendor_home, named_enabled)| {
            parent.is_some_and(|parent| {
                (*named_enabled && parent == vendor_home) || parent == vendor_home.join("rules")
            })
        });
    if is_exact_home_surface {
        return true;
    }
    if workspace_root.is_some_and(|root| path.starts_with(root)) {
        return false;
    }
    path.starts_with(axon_home)
        || vendor_homes
            .iter()
            .any(|(vendor_home, _)| path.starts_with(vendor_home))
}

/// Aborts the wrapped tokio task when dropped.
///
/// Use to tie a spawned helper task's lifetime to an async scope so that
/// cancelling the parent future (e.g. a turn abort dropping the tool loop)
/// also tears down the helper instead of leaving it running detached.
/// Aborting an already-finished task is a no-op, so this is safe to hold
/// across normal scope exit too.
pub struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod is_user_instruction_path_tests {
    use super::is_user_instruction_path;
    use std::path::Path;

    #[test]
    fn axon_home_named_file_nested_in_workspace_is_user_scoped() {
        assert!(is_user_instruction_path(
            Path::new("/repo/config/AGENTS.md"),
            Path::new("/repo/config"),
            &[],
            Some(Path::new("/repo")),
        ));
        assert!(!is_user_instruction_path(
            Path::new("/repo/config/src/AGENTS.md"),
            Path::new("/repo/config"),
            &[],
            Some(Path::new("/repo")),
        ));
    }

    #[test]
    fn workspace_descendants_under_axon_home_stay_project_scoped() {
        assert!(!is_user_instruction_path(
            Path::new("/custom/axon/worktrees/repo/src/AGENTS.md"),
            Path::new("/custom/axon"),
            &[],
            Some(Path::new("/custom/axon/worktrees/repo")),
        ));
    }
}
