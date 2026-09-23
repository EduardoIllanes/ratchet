//! `ratchet config init`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The repo marker template (spec §4.3), with the two optional keys commented out.
const TEMPLATE: &str = r#"# ratchet repo marker. Its presence opts this repo in; without it every hook is a no-op.

[repo]
# name = "myproject"          # optional; defaults to the directory name
default_branch = "main"
worktrees_dir = ".worktrees"  # relative to the repo root; writes here are never "main tree"

[guardrails]
off = []                       # ids of built-in rules to disable in this repo
# extra = "ratchet/guardrails.toml"   # optional: repo-specific rules, same schema as built-ins

[thresholds]                   # optional, defaults shown
live_minutes = 10
idle_minutes = 60
"#;

/// Writes a commented `ratchet.toml` at the root of the git repository containing `cwd`.
///
/// `repo::find_repo` resolves a repo from the nearest existing marker, which would be
/// circular here: this command's job is to create that marker where none exists yet. So the
/// root is resolved by shelling out to git (`git rev-parse --show-toplevel`), the same way
/// `repo::git_branch` and `tests/spec/support.rs::git` do.
pub fn init(force: bool, cwd: &Path) -> i32 {
    let root = match git_toplevel(cwd) {
        Some(r) => r,
        None => {
            eprintln!("not a git repository: {}", cwd.display());
            return 1;
        }
    };
    let path = root.join("ratchet.toml");
    if path.is_file() && !force {
        eprintln!(
            "ratchet.toml already exists at {}; use --force to overwrite",
            path.display()
        );
        return 1;
    }
    if let Err(e) = std::fs::write(&path, TEMPLATE) {
        eprintln!("error: {e}");
        return 1;
    }
    println!("wrote {}", path.display());
    0
}

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["-C", &cwd.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    Some(native_path(&PathBuf::from(s)))
}

/// Rebuilds `p` using this platform's native separator, without touching case or resolving
/// symlinks (unlike `repo::normalize`, which does both and is meant for comparison, not
/// display/writing). Git always prints `/`-separated paths, even on Windows (`rev-parse
/// --show-toplevel`, `--git-common-dir`, `worktree list --porcelain`, ...); joining that
/// straight into a `PathBuf` with `Path::join` leaves the git-printed prefix `/`-separated
/// while the joined suffix picks up `\`, so the result mixes separators when printed or
/// written to a file. `Path::components()` parses both `/` and `\` as separators on Windows, so
/// collecting them back into a fresh `PathBuf` re-renders the whole path with the native
/// separator. Deliberately not `fs::canonicalize`, which on Windows prefixes the result with
/// `\\?\` (a verbatim path other tools, and the tests here, do not expect).
fn native_path(p: &Path) -> PathBuf {
    p.components().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RepoConfig;

    #[test]
    fn template_parses_with_default_branch_main() {
        let cfg: RepoConfig = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(cfg.repo.default_branch.as_deref(), Some("main"));
    }

    /// The scenario CI actually hit on Windows: git prints a `/`-separated absolute path
    /// (drive letter included), which must come back fully native-separated, not mixed.
    ///
    /// The expected value is built as a literal string, not via repeated `Path::join`: joining
    /// onto a bare drive prefix like `Path::new("C:")` does not insert a separator (`"C:"
    /// .join("Users")` is the drive-relative path `"C:Users"`, not `"C:\Users"`), so that
    /// construction would silently assert the wrong thing on Windows.
    #[test]
    fn native_path_rebuilds_a_git_style_forward_slash_path_natively() {
        let git_printed = Path::new("C:/Users/runneradmin/AppData/Local/Temp/.tmpvLAwlO");
        let expected = if cfg!(windows) {
            PathBuf::from(r"C:\Users\runneradmin\AppData\Local\Temp\.tmpvLAwlO")
        } else {
            // No drive-letter concept off Windows: `/` is already the native separator, so the
            // git-printed form is left as-is.
            git_printed.to_path_buf()
        };
        assert_eq!(native_path(git_printed), expected);
    }

    /// A path with no separators at all to normalize is left alone.
    #[test]
    fn native_path_is_a_no_op_on_a_relative_single_component() {
        assert_eq!(
            native_path(Path::new("ratchet.toml")),
            Path::new("ratchet.toml")
        );
    }
}
