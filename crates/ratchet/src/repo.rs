//! Which repo does a working directory belong to? Answered from the nearest `ratchet.toml`,
//! without spawning git: a linked worktree has a `.git` *file* pointing at the main checkout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{self, ConfigError, RepoConfig, MARKER};

#[derive(Debug, Clone)]
#[allow(dead_code)] // checkout_root/name/config consumed by later tasks (guardrails, hooks)
pub struct Repo {
    /// The main checkout (owner of the worktrees).
    pub main_root: PathBuf,
    /// Where the marker nearest to cwd was found (a worktree or the main root).
    pub checkout_root: PathBuf,
    pub name: String,
    pub worktrees_dir: PathBuf,
    pub config: RepoConfig,
}

// Consumed by Task 9 (hooks::dispatch) and Task 10 (guardrails::cli).
#[allow(dead_code)]
pub fn find_repo(cwd: &Path) -> Result<Option<Repo>, ConfigError> {
    let Some(checkout_root) = nearest_marker_dir(cwd) else {
        return Ok(None);
    };
    let main_root = main_root_of(&checkout_root);
    let cfg_path = if main_root.join(MARKER).is_file() {
        main_root.join(MARKER)
    } else {
        checkout_root.join(MARKER)
    };
    let cfg = config::load_repo_config(&cfg_path)?;
    let name = cfg.repo.name.clone().unwrap_or_else(|| {
        main_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repo".into())
    });
    let worktrees_dir = main_root.join(cfg.repo.worktrees_dir.as_deref().unwrap_or(".worktrees"));
    Ok(Some(Repo {
        main_root,
        checkout_root,
        name,
        worktrees_dir,
        config: cfg,
    }))
}

// Internal to find_repo above; reachable once Task 9/10 call it.
#[allow(dead_code)]
fn nearest_marker_dir(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|d| d.join(MARKER).is_file())
        .map(Path::to_path_buf)
}

/// `<checkout>/.git` as a file means a linked worktree: `gitdir: <main>/.git/worktrees/<name>`.
// Used by find_repo above; reachable once Task 9/10 call it.
#[allow(dead_code)]
pub fn main_root_of(checkout_root: &Path) -> PathBuf {
    let dot_git = checkout_root.join(".git");
    if dot_git.is_file() {
        if let Ok(text) = fs::read_to_string(&dot_git) {
            if let Some(rest) = text.trim().strip_prefix("gitdir:") {
                let gitdir = PathBuf::from(rest.trim());
                let gitdir = if gitdir.is_absolute() {
                    gitdir
                } else {
                    checkout_root.join(gitdir)
                };
                // <main>/.git/worktrees/<name> → <main>
                if let Some(main_git) = gitdir.parent().and_then(Path::parent) {
                    if main_git.file_name().map(|n| n == ".git").unwrap_or(false) {
                        if let Some(main) = main_git.parent() {
                            return main.to_path_buf();
                        }
                    }
                }
            }
        }
    }
    checkout_root.to_path_buf()
}

/// Absolute, symlink-resolved when possible, lower-cased, without the Windows `\\?\` prefix.
///
/// `target` need not exist on disk (e.g. a file about to be written): when `p` itself can't be
/// canonicalized, we canonicalize its longest existing ancestor and re-append the missing tail
/// components, rather than falling back to `std::path::absolute` for the whole path. That keeps
/// `within(target, root)` correct even when `root` canonicalizes to a different spelling than its
/// literal form (Windows 8.3 short names, a symlinked temp dir). Only when no ancestor exists at
/// all do we fall back to `std::path::absolute`.
pub fn normalize(p: &Path) -> PathBuf {
    let abs = resolve(p);
    let s = abs.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    PathBuf::from(s.to_lowercase().replace('/', std::path::MAIN_SEPARATOR_STR))
}

fn resolve(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    for ancestor in p.ancestors().skip(1) {
        if let Ok(canon) = ancestor.canonicalize() {
            if let Ok(tail) = p.strip_prefix(ancestor) {
                return canon.join(tail);
            }
        }
    }
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

// Consumed by Task 7 (guardrails::eval) and Task 8 (guardrails::main_tree).
#[allow(dead_code)]
pub fn within(target: &Path, root: &Path) -> bool {
    let (t, r) = (normalize(target), normalize(root));
    t == r || t.starts_with(&r)
}

/// `.venv` in cwd or any parent up to (and including) the main root.
// Consumed by Task 9 (hooks::dispatch) to build GuardContext.
#[allow(dead_code)]
pub fn has_venv(cwd: &Path, main_root: &Path) -> bool {
    let stop = normalize(main_root);
    for dir in cwd.ancestors() {
        if dir.join(".venv").exists() {
            return true;
        }
        if normalize(dir) == stop {
            break;
        }
    }
    main_root.join(".venv").exists()
}

/// Is `target` tracked by the main checkout? The only subprocess on the hot path, reached only
/// when a write already points inside the main tree.
#[allow(dead_code)] // consumed by the guardrail rules task
pub fn is_tracked(main_root: &Path, target: &Path) -> bool {
    let rel = match rel_for_git(main_root, target) {
        Some(r) => r,
        None => return false,
    };
    Command::new("git")
        .args([
            "-C",
            &main_root.to_string_lossy(),
            "ls-files",
            "--error-unmatch",
            "--",
        ])
        .arg(rel)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Branch checked out at `cwd`, when git can tell. A detached HEAD and any failure answer `None`.
/// Never reached from `pre-tool`: only the session hooks call it, once per session start.
pub fn git_branch(cwd: &Path) -> Option<String> {
    let dir = cwd.to_string_lossy().to_string();
    let out = Command::new("git")
        .args(["-C", &dir, "rev-parse", "--abbrev-ref", "HEAD"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}

fn pathdiff(target: &Path, root: &Path) -> Option<PathBuf> {
    let (t, r) = (normalize(target), normalize(root));
    t.strip_prefix(&r).ok().map(Path::to_path_buf)
}

/// Same resolution as `normalize` (longest-existing-ancestor canonicalization, `\\?\` stripped)
/// but WITHOUT lower-casing, so callers that need the on-disk spelling — chiefly `git`, which is
/// case-sensitive on Linux and macOS — don't get a folded path.
fn canonical_case_preserving(p: &Path) -> PathBuf {
    let abs = resolve(p);
    let s = abs.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    PathBuf::from(s)
}

/// The path to hand `git`, relative to `root`, preserving case. `normalize`/`pathdiff` lower-case
/// for case-insensitive comparisons (`within`, `has_venv`), which is correct there but would send
/// `git ls-files` a folded path — harmless on Windows (case-insensitive FS, `core.ignorecase`)
/// but wrong on a case-sensitive Linux/macOS checkout, where a tracked `Mixed/CaseFile.txt` would
/// be reported untracked. Falls back to the lower-cased diff only when the case-preserving forms
/// don't share a prefix (e.g. `root` resolves to a differently-cased ancestor than `target`'s).
pub(crate) fn rel_for_git(root: &Path, target: &Path) -> Option<PathBuf> {
    let (t, r) = (
        canonical_case_preserving(target),
        canonical_case_preserving(root),
    );
    if let Ok(rel) = t.strip_prefix(&r) {
        return Some(rel.to_path_buf());
    }
    pathdiff(target, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn marker(dir: &Path, body: &str) {
        fs::write(dir.join(crate::config::MARKER), body).unwrap();
    }

    #[test]
    fn no_marker_is_none() {
        let d = tempfile::TempDir::new().unwrap();
        assert!(find_repo(d.path()).unwrap().is_none());
    }

    #[test]
    fn nearest_marker_from_subdir_and_defaults() {
        let d = tempfile::TempDir::new().unwrap();
        marker(d.path(), "");
        let deep = d.path().join("a/b");
        fs::create_dir_all(&deep).unwrap();
        let r = find_repo(&deep).unwrap().unwrap();
        assert_eq!(normalize(&r.main_root), normalize(d.path()));
        assert_eq!(
            normalize(&r.worktrees_dir),
            normalize(&d.path().join(".worktrees"))
        );
        assert_eq!(r.name, d.path().file_name().unwrap().to_string_lossy());
    }

    #[test]
    fn linked_worktree_resolves_to_main_root() {
        let main = tempfile::TempDir::new().unwrap();
        marker(main.path(), "[repo]\nname = \"m\"\n");
        fs::create_dir_all(main.path().join(".git/worktrees/wt")).unwrap();
        let wt = main.path().join(".worktrees/wt");
        fs::create_dir_all(&wt).unwrap();
        marker(&wt, "");
        fs::write(
            wt.join(".git"),
            format!(
                "gitdir: {}\n",
                main.path().join(".git/worktrees/wt").display()
            ),
        )
        .unwrap();
        let r = find_repo(&wt).unwrap().unwrap();
        assert_eq!(normalize(&r.main_root), normalize(main.path()));
        assert_eq!(normalize(&r.checkout_root), normalize(&wt));
        assert_eq!(r.name, "m");
    }

    #[test]
    fn invalid_marker_is_an_error() {
        let d = tempfile::TempDir::new().unwrap();
        marker(d.path(), "[repo\n");
        assert!(find_repo(d.path()).is_err());
    }

    #[test]
    fn within_is_case_insensitive_and_prefix_safe() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path().join("Repo");
        fs::create_dir_all(root.join("src")).unwrap();
        assert!(within(&root.join("src/x.rs"), &root));
        let upper = PathBuf::from(root.to_string_lossy().to_uppercase());
        assert!(within(&root.join("src"), &upper));
        assert!(!within(&d.path().join("Repo2/x"), &root));
    }

    #[test]
    fn has_venv_walks_up_to_main_root() {
        let d = tempfile::TempDir::new().unwrap();
        let deep = d.path().join("a/b");
        fs::create_dir_all(&deep).unwrap();
        assert!(!has_venv(&deep, d.path()));
        fs::create_dir_all(d.path().join(".venv")).unwrap();
        assert!(has_venv(&deep, d.path()));
    }

    /// R4: a target that does not exist yet (a file about to be written) must still normalize
    /// under an existing root's canonical form, so `within` stays correct for it.
    #[test]
    fn within_true_for_nonexistent_target_under_existing_root() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path().join("Root");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("brand_new_file.rs");
        assert!(!target.exists());
        assert!(within(&target, &root));
        assert_eq!(
            normalize(&target),
            normalize(&root).join("brand_new_file.rs")
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(st.success());
    }

    /// Fix round 1: `git` is case-sensitive on Linux/macOS, so the relative path handed to it
    /// must preserve case even though `normalize()` (used for `within`/`has_venv` comparisons)
    /// lower-cases everything.
    #[test]
    fn is_tracked_preserves_case_for_git() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path();
        git(root, &["init", "-q"]);
        fs::create_dir_all(root.join("Mixed")).unwrap();
        fs::write(root.join("Mixed/CaseFile.txt"), "x").unwrap();
        git(root, &["add", "Mixed/CaseFile.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        let target = root.join("Mixed/CaseFile.txt");
        assert!(is_tracked(root, &target));

        let rel = rel_for_git(root, &target).unwrap();
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        assert_eq!(parts, vec!["Mixed", "CaseFile.txt"]);
    }
}
