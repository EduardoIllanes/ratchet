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
    append_agents_block(&root)
}

/// Appends `AGENTS_BLOCK` to `<root>/CLAUDE.md` unless a `<!-- ratchet agents:` line is already
/// there; creates the file when absent. Runs only after `ratchet.toml` is written.
fn append_agents_block(root: &Path) -> i32 {
    let path = root.join("CLAUDE.md");
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.file_type().is_file() {
            eprintln!("CLAUDE.md is not a regular file; the agents block was not added");
            return 0;
        }
    }
    let existing = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            eprintln!("error: could not read CLAUDE.md: {e}");
            return 1;
        }
    };
    if String::from_utf8_lossy(&existing)
        .lines()
        .any(|l| l.starts_with("<!-- ratchet agents:"))
    {
        return 0;
    }
    let mut addition = String::new();
    if !existing.is_empty() {
        if !existing.ends_with(b"\n") {
            addition.push('\n');
        }
        addition.push('\n');
    }
    addition.push_str(AGENTS_BLOCK);
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, addition.as_bytes()));
    if let Err(e) = written {
        eprintln!("error: could not write CLAUDE.md: {e}");
        return 1;
    }
    println!("wired: CLAUDE.md now says when to dispatch reader and researcher");
    0
}

/// The agents block `init` appends to the repo root's `CLAUDE.md`; fixed text, owned by the user once written.
const AGENTS_BLOCK: &str = r#"<!-- ratchet agents: when to dispatch reader and researcher. Added once by `ratchet config init`; edit freely, it is never rewritten. -->
## ratchet: when to dispatch `reader` and `researcher`

Both keep raw content out of your context: they read, you get the answer. A dispatch costs a
short prompt and a few seconds; a big file read whole costs its full length in your context for
the rest of the session. Choose on that trade.

**`reader`** (haiku, low effort): big file(s) plus one narrow question; cited `path:line` bullets come back.
- Dispatch it when the answer is somewhere in a file past the `big-read` threshold (350 lines by
  default) and you cannot yet say where: a long module, a generated schema, a lockfile, a CI or
  test log, vendored code.
- Dispatch it when `big-read` blocks a whole-file read and you would otherwise guess at
  `offset`/`limit` windows one after another.
- One dispatch for one question across several files ("which of these modules writes the
  cache?"); separate dispatches in parallel for unrelated questions.
- Ask a question, not "summarize this": "where is the retry limit set and what reads it?" gets
  an answer; "what is in here?" gets a table of contents.
- Skip it when `grep` finds the line or you already know the range: `Read` with
  `offset`/`limit` is cheaper than a dispatch.
- Skip it for a file you are about to edit: you need the exact lines yourself, so read the
  window you will change.
- Skip it for a file under the threshold: read it directly.

**`researcher`** (sonnet): facts out of local PDFs, each with a verbatim quote and its page, left as a board note.
- Dispatch it when the work depends on a PDF already on disk: a spec or RFC, a datasheet, a
  paper, a contract, a vendor manual, a regulation.
- Dispatch it instead of running `ratchet pdf` and reading the extract yourself; an extract can
  run to hundreds of KB.
- Give it the path(s), one concrete question, and the task id the brief should land on.
- Skip it when there is no PDF: it reads nothing else, not markdown, not code, not the web.
- Skip it when the text is already extracted under `~/.ratchet/out/pdf/` and one `grep` there
  answers the question.
"#;

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

    #[test]
    fn agents_block_opens_with_the_marker_line() {
        let first = AGENTS_BLOCK.lines().next().unwrap();
        assert!(first.starts_with("<!-- ratchet agents:"));
        assert!(first.contains("reader") && first.contains("researcher"));
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
