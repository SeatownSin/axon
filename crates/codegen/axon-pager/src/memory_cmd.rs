use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use axon_shell::session::memory::storage::MemoryStorage;
use clap::Subcommand;

#[derive(Debug, clap::Args, Clone)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub enum MemoryCommand {
    /// Clear memory files (workspace by default)
    Clear {
        /// Clear workspace-scoped memory (MEMORY.md, sessions/, index.sqlite)
        #[arg(long, group = "scope")]
        workspace: bool,
        /// Clear global MEMORY.md
        #[arg(long, group = "scope")]
        global: bool,
        /// Clear both workspace and global memory
        #[arg(long, group = "scope")]
        all: bool,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Open a memory file in $VISUAL / $EDITOR (workspace by default)
    Edit {
        /// Edit the global MEMORY.md instead of this workspace's
        #[arg(long)]
        global: bool,
    },
    /// Show memory statistics: file counts, indexed chunks, and index size
    Stats,
}

struct ClearTarget {
    label: &'static str,
    path: PathBuf,
    clear: fn(&MemoryStorage) -> std::io::Result<bool>,
}

fn workspace_target(storage: &MemoryStorage) -> ClearTarget {
    ClearTarget {
        label: "workspace memory",
        path: storage.workspace_dir().to_path_buf(),
        clear: |s| s.clear_workspace(),
    }
}

fn global_target(storage: &MemoryStorage) -> ClearTarget {
    ClearTarget {
        label: "global MEMORY.md",
        path: storage.global_memory_file(),
        clear: |s| s.clear_global(),
    }
}

pub fn run(args: MemoryArgs) -> Result<()> {
    match args.command {
        MemoryCommand::Clear {
            global, all, yes, ..
        } => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            let storage = MemoryStorage::new(&cwd, None);

            let targets = if all {
                vec![workspace_target(&storage), global_target(&storage)]
            } else if global {
                vec![global_target(&storage)]
            } else {
                vec![workspace_target(&storage)]
            };

            run_clear(&storage, &targets, yes)
        }
        MemoryCommand::Edit { global } => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            let storage = MemoryStorage::new(&cwd, None);
            if global {
                run_edit(storage.global_memory_file(), "global MEMORY.md")
            } else {
                run_edit(storage.workspace_memory_file(), "workspace MEMORY.md")
            }
        }
        MemoryCommand::Stats => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            let storage = MemoryStorage::new(&cwd, None);
            run_stats(&storage)
        }
    }
}

fn run_clear(storage: &MemoryStorage, targets: &[ClearTarget], skip_confirm: bool) -> Result<()> {
    let existing: Vec<_> = targets.iter().filter(|t| t.path.exists()).collect();

    if existing.is_empty() {
        println!("Nothing to clear \u{2014} no memory files found.");
        return Ok(());
    }

    println!("The following will be deleted:");
    for t in &existing {
        println!("  {}: {}", t.label, t.path.display());
    }

    if !skip_confirm {
        print!("\nAre you sure? [y/N] ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let mut cleared = false;
    let mut errors: Vec<String> = Vec::new();
    for t in targets {
        match (t.clear)(storage) {
            Ok(true) => {
                cleared = true;
                println!("  Cleared: {}", t.label);
            }
            Ok(false) => {} // nothing to clear for this scope
            Err(e) => {
                errors.push(format!("{}: {e}", t.label));
            }
        }
    }

    if cleared && errors.is_empty() {
        println!("Memory cleared.");
    } else if cleared {
        println!("Memory partially cleared. Errors:");
        for e in &errors {
            eprintln!("  {e}");
        }
    } else if !errors.is_empty() {
        eprintln!("Failed to clear memory:");
        for e in &errors {
            eprintln!("  {e}");
        }
        return Err(anyhow::anyhow!("clear failed"));
    }

    Ok(())
}

/// Resolve the editor to spawn, in the order a user expects it to be honoured.
///
/// `$VISUAL` before `$EDITOR` is the long-standing convention: `VISUAL` names a
/// full-screen editor and `EDITOR` may legitimately be a line editor. Falling
/// back to Notepad rather than vi on Windows because vi is not there.
fn resolve_editor() -> String {
    for key in ["VISUAL", "EDITOR"] {
        if let Some(value) = std::env::var_os(key) {
            let value = value.to_string_lossy().trim().to_string();
            if !value.is_empty() {
                return value;
            }
        }
    }
    if cfg!(windows) { "notepad" } else { "vi" }.to_string()
}

/// Open a memory file in the user's editor, creating it if absent.
///
/// Creating it matters: memory files are written lazily by a session, so on a
/// workspace that has not run with `--experimental-memory` yet there is nothing
/// to open. Spawning an editor on a missing path gives a different error per
/// editor; an empty file is the outcome the user asked for either way.
fn run_edit(path: PathBuf, label: &str) -> Result<()> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create memory directory {}", parent.display())
            })?;
        }
        std::fs::File::create(&path)
            .with_context(|| format!("failed to create {}", path.display()))?;
        println!("Created empty {label}: {}", path.display());
    }

    // Split on whitespace so `EDITOR="code --wait"` works; the editor name is
    // a user-controlled command by design, exactly as with `git`'s core.editor.
    let editor = resolve_editor();
    let mut parts = editor.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty editor command"))?;

    let status = std::process::Command::new(program)
        .args(parts)
        .arg(&path)
        .status()
        .with_context(|| format!("failed to launch editor '{program}'"))?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "editor '{program}' exited with {}",
            status
                .code()
                .map_or_else(|| "a signal".to_string(), |c| format!("status {c}"))
        ));
    }
    Ok(())
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Render a byte count the way `ls -h` would, so a 10 MB index is obvious at a
/// glance rather than being an eleven-digit number.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Report what memory is on disk: file counts by source, indexed chunks, and
/// the size of the index.
///
/// Every number here is read from disk at call time rather than from a running
/// session, so it is safe to run while Axon is open and it reports what a new
/// session would actually load.
fn run_stats(storage: &MemoryStorage) -> Result<()> {
    let files = storage
        .list_memory_files()
        .context("failed to list memory files")?;

    let mut counts = [("global", 0usize), ("workspace", 0), ("session", 0)];
    let mut total_bytes = 0u64;
    for path in &files {
        let source = storage.classify_source(path);
        if let Some(entry) = counts.iter_mut().find(|(name, _)| *name == source) {
            entry.1 += 1;
        }
        total_bytes += file_len(path);
    }

    let index_path = storage.workspace_dir().join("index.sqlite");
    let index_exists = index_path.is_file();

    println!(
        "Global memory:    {}",
        storage.global_memory_file().display()
    );
    println!("Workspace memory: {}", storage.workspace_dir().display());
    if storage.is_ephemeral() {
        // An ephemeral workspace is a temp dir that will not survive, so its
        // counts describe nothing durable. Say so rather than letting the
        // numbers imply persistence.
        println!("                  (ephemeral — this workspace's memory is not persisted)");
    }
    println!();

    println!("Files: {} total", files.len());
    for (label, count) in counts {
        println!("  {label:<10} {count}");
    }
    println!("  {:<10} {}", "size", human_bytes(total_bytes));
    println!();

    if index_exists {
        println!("Index: {}", index_path.display());
        println!("  {:<10} {}", "chunks", storage.total_chunk_count());
        println!("  {:<10} {}", "size", human_bytes(file_len(&index_path)));
    } else {
        // Not an error: the index is built by a session running with memory
        // enabled, so its absence is the normal state before first use.
        println!("Index: none yet at {}", index_path.display());
        println!("  built on the first session run with --experimental-memory");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    /// Parse `axon memory <args>` the way the real CLI does.
    #[derive(clap::Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: MemoryCommand,
    }

    fn parse(args: &[&str]) -> MemoryCommand {
        TestCli::parse_from(std::iter::once("memory").chain(args.iter().copied())).command
    }

    #[test]
    fn edit_defaults_to_workspace_scope() {
        assert!(matches!(
            parse(&["edit"]),
            MemoryCommand::Edit { global: false }
        ));
    }

    #[test]
    fn edit_global_selects_global_scope() {
        assert!(matches!(
            parse(&["edit", "--global"]),
            MemoryCommand::Edit { global: true }
        ));
    }

    #[test]
    fn stats_takes_no_arguments() {
        assert!(matches!(parse(&["stats"]), MemoryCommand::Stats));
        assert!(TestCli::try_parse_from(["memory", "stats", "--global"]).is_err());
    }

    /// The variants documented in the README must be the ones clap accepts.
    /// This file previously offered only `clear`, while the README advertised
    /// `edit`, `edit --global` and `stats` — the drift this test exists to stop.
    #[test]
    fn documented_subcommands_all_parse() {
        for args in [
            vec!["clear"],
            vec!["clear", "--global"],
            vec!["clear", "--all", "--yes"],
            vec!["edit"],
            vec!["edit", "--global"],
            vec!["stats"],
        ] {
            assert!(
                TestCli::try_parse_from(std::iter::once("memory").chain(args.iter().copied()))
                    .is_ok(),
                "documented invocation `axon memory {}` does not parse",
                args.join(" ")
            );
        }
    }

    #[test]
    fn human_bytes_uses_binary_units_and_scales() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        // Exactly at the boundary it promotes, rather than printing "1024 B".
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn human_bytes_saturates_at_the_largest_unit() {
        // Beyond GB there is no larger unit, so the number grows rather than
        // the loop running off the end of UNITS.
        assert_eq!(human_bytes(5000u64 * 1024 * 1024 * 1024), "5000.0 GB");
    }

    #[test]
    fn file_len_of_a_missing_path_is_zero() {
        // stats must total a partially-present memory dir without erroring.
        assert_eq!(file_len(Path::new("does/not/exist/anywhere.md")), 0);
    }
}
