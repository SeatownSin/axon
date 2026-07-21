//! `AXON_HOME` override tests in an isolated binary so `axon_home()`'s
//! process-wide `OnceLock` initializes from the overridden env var.

use std::path::PathBuf;

#[test]
fn axon_home_override_path_helpers() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let axon_home = tmp.path().to_path_buf();
    unsafe {
        std::env::set_var("AXON_HOME", &axon_home);
    }

    assert_eq!(
        axon_pager::util::pager_toml_path(),
        axon_home.join("pager.toml")
    );
    assert_eq!(
        axon_pager::util::display_axon_home_prefix(),
        "$AXON_HOME"
    );
    assert_eq!(
        axon_pager::util::display_user_axon_path("config.toml"),
        "$AXON_HOME/config.toml"
    );

    let memory_path = axon_home.join("memory/MEMORY.md");
    assert_eq!(
        axon_pager::util::abbreviate_path(&memory_path.display().to_string()),
        "$AXON_HOME/memory/MEMORY.md"
    );

    assert!(axon_pager::util::is_under_user_axon_home(&memory_path));
    assert!(!axon_pager::util::is_under_user_axon_home(
        PathBuf::from("/tmp/other").as_path()
    ));
}
