//! Shared resolution of a tool-input path (a `Read`'s `file_path`, an `Edit`/`Write`'s
//! `file_path`/`notebook_path`, or a token pulled out of a `Bash`/`PowerShell` command) into the
//! `PathBuf` a guardrail rule reasons about. Used by `big_read::is_big_outside_worktree` and
//! `main_tree::resolves_to_tracked_main_tree`, which otherwise each open-coded the same
//! `PathBuf::from` + `is_absolute` + `cwd.join` steps.
//!
//! ON WINDOWS ONLY, this also reads a Git Bash / MSYS style path -- `/c/Users/...` or
//! `/cygdrive/c/Users/...`, including the bare `/c` form -- as the Windows path it names,
//! `C:/Users/...`, before the absolute/relative decision. Such a path is not `is_absolute()` on
//! Windows (no drive letter), so a tool call that actually runs through `bash.exe` would
//! otherwise get silently joined onto `cwd` and never resolve to the real file (T-0020). On
//! every other platform a leading `/` is already an absolute path and nothing is translated.
//! `[guardrails] big_read_lines` and every other threshold are unaffected by this translation --
//! it only changes which `PathBuf` a rule inspects.

use std::path::{Path, PathBuf};

/// `raw` (a tool-input path, possibly relative, possibly Git-Bash/MSYS form on Windows) resolved
/// against `cwd` into the absolute `PathBuf` a guardrail rule should inspect. `windows` is
/// threaded in explicitly rather than read from `cfg!(windows)` here so the Windows-only
/// translation stays a pure, deterministically testable function; real callers pass
/// `cfg!(windows)`.
pub fn resolve_tool_path(raw: &str, cwd: &Path, windows: bool) -> PathBuf {
    let translated = if windows {
        translate_git_bash_drive(raw)
    } else {
        raw.to_string()
    };
    let target = PathBuf::from(translated);
    if target.is_absolute() {
        target
    } else {
        cwd.join(target)
    }
}

/// `/<letter>/rest`, `/<letter>` (bare, no `rest`) or `/cygdrive/<letter>/rest` -> `<LETTER>:/rest`
/// (`<LETTER>:/` for the bare form). Anything else -- a relative path, a path already in Windows
/// form, a leading segment that is not exactly one ASCII letter (`/cc/x`, `/1/x`) -- is returned
/// unchanged. Pure string transform, no filesystem access, so it is exercised on every platform
/// even though it is only ever called with `windows: true`.
fn translate_git_bash_drive(raw: &str) -> String {
    let Some(rest) = raw
        .strip_prefix("/cygdrive/")
        .or_else(|| raw.strip_prefix('/'))
    else {
        return raw.to_string();
    };
    let mut chars = rest.chars();
    let Some(letter) = chars.next() else {
        return raw.to_string();
    };
    if !letter.is_ascii_alphabetic() {
        return raw.to_string();
    }
    match chars.next() {
        None => format!("{}:/", letter.to_ascii_uppercase()),
        Some('/') => format!("{}:/{}", letter.to_ascii_uppercase(), chars.as_str()),
        // A second character that is not `/`: the leading segment is not a single drive
        // letter (`/cc/x`), so this is not a Git Bash drive path at all.
        Some(_) => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_drive_letter_translates() {
        assert_eq!(translate_git_bash_drive("/c/x"), "C:/x");
    }

    #[test]
    fn uppercase_drive_letter_translates() {
        assert_eq!(translate_git_bash_drive("/C/x"), "C:/x");
    }

    #[test]
    fn cygdrive_form_translates() {
        assert_eq!(translate_git_bash_drive("/cygdrive/d/x"), "D:/x");
    }

    #[test]
    fn bare_drive_letter_translates_to_root() {
        assert_eq!(translate_git_bash_drive("/c"), "C:/");
    }

    #[test]
    fn two_letter_segment_is_not_a_drive() {
        assert_eq!(translate_git_bash_drive("/cc/x"), "/cc/x");
    }

    #[test]
    fn digit_segment_is_not_a_drive() {
        assert_eq!(translate_git_bash_drive("/1/x"), "/1/x");
    }

    #[test]
    fn relative_path_is_untouched() {
        assert_eq!(translate_git_bash_drive("src/foo.rs"), "src/foo.rs");
    }

    #[test]
    fn already_windows_path_is_untouched() {
        assert_eq!(
            translate_git_bash_drive("C:/Users/x/big.py"),
            "C:/Users/x/big.py"
        );
        assert_eq!(
            translate_git_bash_drive("C:\\Users\\x\\big.py"),
            "C:\\Users\\x\\big.py"
        );
    }

    #[test]
    fn unix_no_op_regardless_of_shape() {
        // windows: false must never translate, even a shape that looks like a drive path.
        assert_eq!(
            resolve_tool_path("/c/x", Path::new("/repo"), false),
            PathBuf::from("/c/x")
        );
    }

    // `PathBuf::is_absolute()` uses the *build target's* native rules, not the `windows: bool`
    // argument -- on a non-Windows build "C:/Users/x" is never absolute no matter what is
    // passed in, so this assertion only holds when actually compiled for Windows. The
    // translation itself (`translate_git_bash_drive`, above) is still checked on every
    // platform; only the "and is now recognised as absolute" half is Windows-only, matching the
    // `#[cfg(windows)]` scenario tests in agent_protocol.rs that exercise this end-to-end.
    #[test]
    #[cfg(windows)]
    fn resolve_windows_drive_path_is_absolute_after_translation() {
        assert_eq!(
            resolve_tool_path("/c/Users/x/big.py", Path::new("C:/repo"), true),
            PathBuf::from("C:/Users/x/big.py")
        );
        assert_eq!(
            resolve_tool_path("/cygdrive/c/Users/x/big.py", Path::new("C:/repo"), true),
            PathBuf::from("C:/Users/x/big.py")
        );
    }

    #[test]
    fn resolve_relative_path_joins_cwd_on_windows_too() {
        assert_eq!(
            resolve_tool_path("big.py", Path::new("C:/repo"), true),
            PathBuf::from("C:/repo").join("big.py")
        );
    }

    #[test]
    fn resolve_relative_path_joins_cwd_on_unix() {
        assert_eq!(
            resolve_tool_path("big.py", Path::new("/repo"), false),
            PathBuf::from("/repo").join("big.py")
        );
    }
}
