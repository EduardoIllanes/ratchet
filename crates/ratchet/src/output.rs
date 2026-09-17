//! What a face prints. One rule (spec §4.1): more than 60 lines is not something to read in a
//! terminal, so it goes to a file under the state directory and the terminal gets the first 20
//! plus the path. Hooks are exempt — the briefing has its own 40-line cap and is context, not a
//! listing.

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::model::Task;

// Consumed by cli::task_cmd (Task 8) and hooks::briefing (Task 9).
#[allow(dead_code)]
pub const MAX_STDOUT_LINES: usize = 60;
// Consumed by cli::task_cmd (Task 8) and hooks::briefing (Task 9).
#[allow(dead_code)]
pub const HEAD_LINES: usize = 20;

// Consumed by cli::task_cmd (Task 8) and hooks::briefing (Task 9).
#[allow(dead_code)]
pub fn out_dir(home: &Path) -> PathBuf {
    home.join("out")
}

/// A filename collision is possible whenever two long outputs land in the same second under a
/// fixed clock (real wall-clock coincidence, or `RATCHET_NOW` pinned for a test/replay): the
/// second call would otherwise overwrite the first's file via `fs::write`'s truncate-and-replace
/// semantics, silently losing it while the terminal still prints its (now wrong) path. Up to
/// this many suffixed candidates are tried before giving up on finding a free name.
const MAX_NAME_ATTEMPTS: u32 = 1000;

/// `<stamp>-<name>.<ext>` for `n == 1`, `<stamp>-<name>-<n>.<ext>` for `n > 1`. The first
/// candidate's shape is unchanged from before collision-safety existed, so existing callers and
/// fixtures that look for `<stamp>-<name>.<ext>` keep working.
fn candidate_path(dir: &Path, stamp: &str, name: &str, ext: &str, n: u32) -> PathBuf {
    if n <= 1 {
        dir.join(format!("{stamp}-{name}.{ext}"))
    } else {
        dir.join(format!("{stamp}-{name}-{n}.{ext}"))
    }
}

/// Outcome of trying to land `body` under a collision-safe name.
enum Written {
    Ok(PathBuf),
    Err { path: PathBuf, err: io::Error },
}

/// Writes `body` under `<dir>/<stamp>-<name>.<ext>`, or the first free `-2`, `-3`, … suffix if
/// that name is already taken. Each attempt opens with `create_new(true)` so the "does it exist"
/// check and the create are one atomic filesystem operation, not a race. If every attempt up to
/// `MAX_NAME_ATTEMPTS` collides, the last candidate is used anyway (best effort, matching the
/// pre-existing truncate-on-write behavior) rather than losing the output entirely.
fn write_collision_safe(dir: &Path, stamp: &str, name: &str, ext: &str, body: &str) -> Written {
    if let Err(err) = fs::create_dir_all(dir) {
        return Written::Err {
            path: candidate_path(dir, stamp, name, ext, 1),
            err,
        };
    }
    let mut last = candidate_path(dir, stamp, name, ext, 1);
    for n in 1..=MAX_NAME_ATTEMPTS {
        let candidate = candidate_path(dir, stamp, name, ext, n);
        last = candidate.clone();
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut f) => {
                return match f.write_all(body.as_bytes()) {
                    Ok(()) => Written::Ok(candidate),
                    Err(err) => Written::Err {
                        path: candidate,
                        err,
                    },
                };
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Written::Err {
                    path: candidate,
                    err,
                }
            }
        }
    }
    // Every candidate up to the bound was taken: fall back to the last one rather than lose the
    // output, matching the plain `fs::write` behavior this function replaces.
    match fs::write(&last, body.as_bytes()) {
        Ok(()) => Written::Ok(last),
        Err(err) => Written::Err { path: last, err },
    }
}

/// Prints `lines`, or writes them to `<home>/out/<stamp>-<name>.txt` (or a collision-safe `-2`,
/// `-3`, … variant if that name is already taken) and prints a window. If the file cannot be
/// written the whole output is printed instead: the discipline may cost a long terminal, never a
/// lost result.
// Consumed by cli::task_cmd (Task 8).
#[allow(dead_code)]
pub fn emit(home: &Path, name: &str, lines: &[String], now: DateTime<Utc>) {
    if lines.len() <= MAX_STDOUT_LINES {
        for line in lines {
            println!("{line}");
        }
        return;
    }
    let dir = out_dir(home);
    let stamp = now.format("%Y%m%dT%H%M%S").to_string();
    let body = format!("{}\n", lines.join("\n"));
    match write_collision_safe(&dir, &stamp, name, "txt", &body) {
        Written::Ok(path) => {
            for line in lines.iter().take(HEAD_LINES) {
                println!("{line}");
            }
            println!("… ({} lines in {})", lines.len(), path.display());
        }
        Written::Err { path, err } => {
            for line in lines {
                println!("{line}");
            }
            eprintln!("warning: could not write {}: {err}", path.display());
        }
    }
}

/// Machine output, under the same limit.
// Consumed by cli::task_cmd (Task 8).
#[allow(dead_code)]
pub fn emit_json(home: &Path, name: &str, value: &serde_json::Value, now: DateTime<Utc>) {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string());
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    emit(home, name, &lines, now);
}

/// The one line a task takes in every listing and in the briefing. A task with no checklist shows
/// no progress at all — not `(0/0)`, which would read as "nothing done" instead of "nothing to
/// do" (spec §4.5).
// Consumed by cli::task_cmd (Task 8) and hooks::briefing (Task 9).
#[allow(dead_code)]
pub fn format_task_line(task: &Task, progress: Option<(i64, i64)>) -> String {
    let progress = progress
        .map(|(done, total)| format!("  ({done}/{total})"))
        .unwrap_or_default();
    // Flatten every whitespace run, including embedded newlines, to a single space: the
    // discipline's line-count accounting (here and in `emit`'s threshold, and the briefing's
    // 40-line cap) counts Vec entries, not physical lines, so a raw title with a newline would
    // silently break "one line per task" (agent-protocol spec).
    let title: String = task.title.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{}  {:<12} p{}  {}{}",
        task.id,
        task.status.as_str(),
        task.priority,
        title,
        progress
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TaskStatus;

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    fn task(id: &str, title: &str) -> Task {
        Task {
            id: id.into(),
            title: title.into(),
            body: String::new(),
            repo: "demo".into(),
            repo_root: "root".into(),
            status: TaskStatus::InProgress,
            priority: 2,
            parent_id: None,
            tags: Vec::new(),
            claimed_by: None,
            created_at: at("2026-09-16T12:00:00Z"),
            updated_at: at("2026-09-16T12:00:00Z"),
            archived_at: None,
        }
    }

    #[test]
    fn a_task_line_carries_id_status_priority_title_and_progress() {
        let line = format_task_line(&task("T-0042", "Port the board"), Some((2, 5)));
        assert!(line.starts_with("T-0042"), "{line}");
        assert!(line.contains("in_progress"), "{line}");
        assert!(line.contains("p2"), "{line}");
        assert!(line.contains("Port the board"), "{line}");
        assert!(line.contains("(2/5)"), "{line}");
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn a_multiline_title_is_flattened_to_a_single_line() {
        let line = format_task_line(&task("T-0044", "first line\nsecond \"quoted\" line"), None);
        assert_eq!(line.lines().count(), 1, "{line}");
        assert!(!line.contains('\n'), "{line}");
        assert!(line.contains("first line second \"quoted\" line"), "{line}");
    }

    #[test]
    fn a_task_with_no_checklist_shows_no_progress_at_all() {
        let line = format_task_line(&task("T-0043", "No criteria"), None);
        assert!(!line.contains('/'), "{line}");
        // Deliberately narrow: `(0/` is the shape that would mean "nothing done"; a bare `0`
        // would also match a priority or an identifier and fail for an unrelated reason.
        assert!(!line.contains("(0/"), "{line}");
    }

    #[test]
    fn short_output_never_touches_the_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let lines: Vec<String> = (1..=60).map(|n| format!("line {n}")).collect();
        emit(dir.path(), "task-list", &lines, at("2026-09-16T12:00:00Z"));
        assert!(
            !out_dir(dir.path()).exists(),
            "a short output created the directory"
        );
    }

    #[test]
    fn long_output_goes_to_a_named_file_under_the_state_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let lines: Vec<String> = (1..=300).map(|n| format!("line {n}")).collect();
        emit(dir.path(), "task-list", &lines, at("2026-09-16T12:00:00Z"));
        let written: Vec<_> = std::fs::read_dir(out_dir(dir.path()))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(written.len(), 1, "{written:?}");
        let name = written[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(name, "20260916T120000-task-list.txt");
        let body = std::fs::read_to_string(&written[0]).unwrap();
        assert_eq!(body.lines().count(), 300);
        assert!(body.ends_with('\n'));
    }

    #[test]
    fn three_long_outputs_at_the_same_instant_each_keep_their_own_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let now = at("2026-09-16T12:00:00Z");
        let calls: Vec<Vec<String>> = (1..=3)
            .map(|call| (1..=70).map(|n| format!("call {call} line {n}")).collect())
            .collect();
        for lines in &calls {
            emit(dir.path(), "task-list", lines, now);
        }

        let out = out_dir(dir.path());
        let mut written: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        written.sort();
        assert_eq!(
            written.len(),
            3,
            "three same-instant, same-name calls should keep three files, not collide: {written:?}"
        );

        // The naming contract: first call keeps the un-suffixed name, later calls at the same
        // instant get `-2`, `-3`, … — each call's own file, not a shared, overwritten one.
        let expected = [
            out.join("20260916T120000-task-list.txt"),
            out.join("20260916T120000-task-list-2.txt"),
            out.join("20260916T120000-task-list-3.txt"),
        ];
        for (path, lines) in expected.iter().zip(calls.iter()) {
            assert!(path.exists(), "missing {path:?}: {written:?}");
            let body = std::fs::read_to_string(path).unwrap();
            assert_eq!(body.lines().count(), 70, "{path:?} lost lines");
            assert_eq!(
                body.lines().next().unwrap(),
                lines[0],
                "{path:?} does not hold its own call's content — the collision overwrote it"
            );
        }
    }

    #[test]
    fn json_output_obeys_the_same_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let big: Vec<i64> = (1..=200).collect();
        emit_json(
            dir.path(),
            "task-list",
            &serde_json::json!(big),
            at("2026-09-16T12:00:00Z"),
        );
        assert!(out_dir(dir.path()).exists());
    }
}
