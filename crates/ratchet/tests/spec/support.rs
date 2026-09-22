//! Sandbox: a temporary state dir, a temporary git repo with a marker and one tracked file.
//! Tests run the real binary as a subprocess, exactly as Claude Code would.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};
use tempfile::TempDir;

pub struct Sandbox {
    pub home: TempDir,
    pub repo: TempDir,
    pub scratchpad: TempDir,
}

impl Sandbox {
    pub fn root(&self) -> PathBuf {
        strip_verbatim(&self.repo.path().canonicalize().unwrap())
    }
    pub fn log_text(&self) -> String {
        fs::read_to_string(self.home.path().join("ratchet.log")).unwrap_or_default()
    }
    pub fn write_marker(&self, body: &str) {
        fs::write(self.repo.path().join("ratchet.toml"), body).unwrap();
    }
    pub fn write(&self, rel: &str, body: &str) -> PathBuf {
        let p = self.repo.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
        p
    }
}

/// `canonicalize()` on Windows returns the verbatim `\\?\...` form, which real Claude Code
/// tool calls never send in a `cwd` field. Strip it so paths built from a canonicalized root
/// (e.g. `Sandbox::root()`) look like what the hook actually receives.
fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p.to_path_buf(),
    }
}

pub fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git available");
    assert!(st.success(), "git {:?} failed", args);
}

/// Opted-in repo: `.venv/`, `ratchet.toml`, `tracked.txt` committed, `.worktrees/wt` linked worktree.
pub fn sandbox() -> Sandbox {
    let sb = Sandbox {
        home: TempDir::new().unwrap(),
        repo: TempDir::new().unwrap(),
        scratchpad: TempDir::new().unwrap(),
    };
    let root = sb.repo.path();
    fs::create_dir_all(root.join(".venv")).unwrap();
    fs::create_dir_all(root.join("src/deep")).unwrap();
    sb.write_marker("[repo]\ndefault_branch = \"main\"\nworktrees_dir = \".worktrees\"\n");
    fs::write(root.join("tracked.txt"), "hello\n").unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "tracked.txt", "ratchet.toml"]);
    git(root, &["commit", "-q", "-m", "init"]);
    git(
        root,
        &["worktree", "add", "-q", ".worktrees/wt", "-b", "wt"],
    );
    sb
}

/// Directory with no marker anywhere above it.
pub fn unmanaged_dir() -> TempDir {
    TempDir::new().unwrap()
}

pub fn ratchet_bin() -> PathBuf {
    assert_cmd::cargo::cargo_bin("ratchet")
}

pub fn hook_in(sb: &Sandbox, event: &str, payload: &Value, cwd: &Path) -> Output {
    run_hook(
        sb.home.path(),
        Some(sb.scratchpad.path()),
        event,
        &payload.to_string(),
        cwd,
    )
}

/// Like `hook_in`, but the process's own current directory (`process_cwd`) is set
/// independently of the `cwd` field already baked into `payload`. Real Claude Code always
/// puts the tool call's cwd in the payload; a test that wants to prove the hook resolves the
/// repo from that field, not from the OS process cwd, spawns the binary from somewhere else
/// (e.g. an unmanaged directory) via this helper.
pub fn hook_in_from(sb: &Sandbox, event: &str, payload: &Value, process_cwd: &Path) -> Output {
    run_hook(
        sb.home.path(),
        Some(sb.scratchpad.path()),
        event,
        &payload.to_string(),
        process_cwd,
    )
}

pub fn run_hook(
    home: &Path,
    scratchpad: Option<&Path>,
    event: &str,
    stdin_text: &str,
    cwd: &Path,
) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.args(["hook", event])
        .current_dir(cwd)
        .env("RATCHET_HOME", home)
        .env_remove("CLAUDE_SCRATCHPAD")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(s) = scratchpad {
        cmd.env("CLAUDE_SCRATCHPAD", s);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_text.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

pub fn bash(command: &str, cwd: &Path) -> Value {
    json!({ "tool_name": "Bash", "tool_input": { "command": command }, "cwd": cwd.to_string_lossy() })
}

pub fn powershell(command: &str, cwd: &Path) -> Value {
    json!({ "tool_name": "PowerShell", "tool_input": { "command": command }, "cwd": cwd.to_string_lossy() })
}

pub fn edit(file_path: &Path, new_string: &str, cwd: &Path) -> Value {
    json!({ "tool_name": "Edit", "tool_input": { "file_path": file_path.to_string_lossy(), "old_string": "x", "new_string": new_string }, "cwd": cwd.to_string_lossy() })
}

pub fn write(file_path: &Path, content: &str, cwd: &Path) -> Value {
    json!({ "tool_name": "Write", "tool_input": { "file_path": file_path.to_string_lossy(), "content": content }, "cwd": cwd.to_string_lossy() })
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

pub fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

pub fn guardrails(sb: &Sandbox, args: &[&str], cwd: &Path) -> Output {
    Command::new(ratchet_bin())
        .arg("guardrails")
        .args(args)
        .current_dir(cwd)
        .env("RATCHET_HOME", sb.home.path())
        .env_remove("CLAUDE_SCRATCHPAD")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap()
}

// --- group 1: state, sessions -------------------------------------------------------------

use rusqlite::Connection;

/// Fixed instant every group-1 scenario starts from. `RATCHET_NOW` replaces the process clock,
/// which is what makes "ninety minutes later" a test instead of a sleep.
pub const T0: &str = "2026-09-16T12:00:00Z";

/// `T0` plus `minutes`, in the same format the binary writes.
pub fn at(minutes: i64) -> String {
    at_secs(minutes * 60)
}

pub fn at_secs(seconds: i64) -> String {
    let base = chrono::DateTime::parse_from_rfc3339(T0).unwrap();
    (base + chrono::Duration::seconds(seconds))
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Payload of a session-scoped hook (no tool call).
pub fn session_payload(session_id: &str, cwd: &Path) -> Value {
    json!({ "session_id": session_id, "cwd": cwd.to_string_lossy() })
}

/// The linked worktree the sandbox creates.
pub fn worktree(sb: &Sandbox) -> PathBuf {
    sb.root().join(".worktrees").join("wt")
}

/// Key the binary stores in `repo_root`: the main root, absolute and lower-cased.
pub fn repo_root_key(sb: &Sandbox) -> String {
    sb.root().to_string_lossy().to_lowercase()
}

/// Run a hook with extra environment on top of the sandbox defaults.
pub fn hook_env(
    sb: &Sandbox,
    event: &str,
    payload: &Value,
    cwd: &Path,
    extra: &[(&str, &str)],
) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.args(["hook", event])
        .current_dir(cwd)
        .env("RATCHET_HOME", sb.home.path())
        .env("CLAUDE_SCRATCHPAD", sb.scratchpad.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("RATCHET_SESSION_MODE")
        .env_remove("RATCHET_LAUNCHED_BY")
        .env_remove("RATCHET_NOW")
        .env_remove("CLAUDE_ENV_FILE")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Run any CLI subcommand with the same isolation.
pub fn cli(sb: &Sandbox, args: &[&str], cwd: &Path, extra: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.args(args)
        .current_dir(cwd)
        .env("RATCHET_HOME", sb.home.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("RATCHET_NOW")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

pub fn db_file(sb: &Sandbox) -> PathBuf {
    sb.home.path().join("ratchet.db")
}

/// Read-only handle on the state database. Tests stand OUTSIDE the binary, so they are allowed
/// to look at (and seed) rows directly; production code never does this outside `services/`.
pub fn db(sb: &Sandbox) -> Connection {
    Connection::open(db_file(sb)).unwrap()
}

/// Seed one task straight into the database: the board CLI arrives with group 2, and the
/// release-of-dead-work scenarios need a claimed task to exist today.
pub fn seed_task(sb: &Sandbox, id: &str, status: &str, claimed_by: Option<&str>) {
    let conn = db(sb);
    conn.execute(
        "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,tags,claimed_by,created_at,updated_at) \
         VALUES (?1,?2,'',?3,?4,?5,3,'[]',?6,?7,?7)",
        rusqlite::params![
            id,
            format!("seeded {id}"),
            sb.root().file_name().unwrap().to_string_lossy(),
            repo_root_key(sb),
            status,
            claimed_by,
            T0
        ],
    )
    .unwrap();
}

/// `(status, claimed_by)` of a task.
pub fn task_row(sb: &Sandbox, id: &str) -> (String, Option<String>) {
    db(sb)
        .query_row(
            "SELECT status, claimed_by FROM tasks WHERE id = ?1",
            rusqlite::params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

pub fn count(sb: &Sandbox, sql: &str, args: &[&str]) -> i64 {
    db(sb)
        .query_row(sql, rusqlite::params_from_iter(args.iter()), |r| r.get(0))
        .unwrap()
}

pub fn last_seen(sb: &Sandbox, session_id: &str) -> String {
    db(sb)
        .query_row(
            "SELECT last_seen FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .unwrap()
}

/// Register a session through the real hook, at a chosen instant.
pub fn start_session(sb: &Sandbox, id: &str, cwd: &Path, when: &str) -> Output {
    hook_env(
        sb,
        "session-start",
        &session_payload(id, cwd),
        cwd,
        &[("RATCHET_NOW", when)],
    )
}

// --- group 3: pdf ---------------------------------------------------------------------------
//
// No test in `pdf.rs` runs the real `liteparse`. `PdfBox` writes a fake `liteparse.cmd`
// (Windows) and a POSIX twin `liteparse`, both driven by files in a "control" directory the
// test also controls, and configures `[pdf] extractor` to point at the fake script BY ITS
// EXACT PATH (never via `PATH`) — see plan ruling GP-R1/GP-R2. This still exercises the real
// `Liteparse::run` subprocess-spawning code in `src/pdf.rs`, just against a fake child process.
//
// Control directory contract (read by the scripts, written by the test):
//   version.txt          text printed for `-V` (default if absent: "liteparse 0.0.0-fake")
//   <noocr|ocr>.txt       text written to `-o <out>` when no `--target-pages` was given
//   <noocr|ocr>-p<range>.txt   text written to `-o <out>` when `--target-pages <range>` was given
//   fail                  present: exit 1, printing this file's content as the extractor's
//                          stated reason (first line only)
//   timeout               present: sleep past the caller's configured timeout, then exit 1
//                          (the caller kills the process first; this file is a fallback)
//   calls.log             appended one line per invocation: "<noocr|ocr> pages=<range or '-'>"

pub struct PdfBox {
    pub home: TempDir,
    pub work: TempDir,
}

impl PdfBox {
    pub fn new() -> Self {
        let pb = PdfBox {
            home: TempDir::new().unwrap(),
            work: TempDir::new().unwrap(),
        };
        fs::create_dir_all(pb.bin_dir()).unwrap();
        fs::create_dir_all(pb.control_dir()).unwrap();
        pb
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.work.path().join("bin")
    }

    pub fn control_dir(&self) -> PathBuf {
        self.work.path().join("control")
    }

    /// A byte sequence that passes the `%PDF-` magic check — enough for every test that is not
    /// specifically about rejecting a non-PDF file.
    pub fn write_pdf(&self, name: &str) -> PathBuf {
        let p = self.work.path().join(name);
        fs::write(&p, b"%PDF-1.4\n%fake pdf body for tests\n").unwrap();
        p
    }

    pub fn write_non_pdf(&self, name: &str) -> PathBuf {
        let p = self.work.path().join(name);
        fs::write(&p, b"this is not a pdf").unwrap();
        p
    }

    /// The exact path `[pdf] extractor` must be set to for the fake script to run on this OS.
    pub fn fake_extractor_path(&self) -> PathBuf {
        if cfg!(windows) {
            self.bin_dir().join("liteparse.cmd")
        } else {
            self.bin_dir().join("liteparse")
        }
    }

    /// Writes both twins. Only the one matching the running OS is ever executed by a test; the
    /// other ships so the fixture works once ratchet builds for other platforms (group 5).
    pub fn install_fake_liteparse(&self) {
        let control = self.control_dir();
        let win_control = control.to_string_lossy().replace('/', "\\");
        fs::write(
            self.bin_dir().join("liteparse.cmd"),
            FAKE_LITEPARSE_CMD.replace("__CONTROL_DIR__", &win_control),
        )
        .unwrap();

        let posix_control = control.to_string_lossy().to_string();
        let sh_path = self.bin_dir().join("liteparse");
        fs::write(
            &sh_path,
            FAKE_LITEPARSE_SH.replace("__CONTROL_DIR__", &posix_control),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&sh_path).unwrap().permissions();
            perm.set_mode(0o755);
            fs::set_permissions(&sh_path, perm).unwrap();
        }
    }

    /// Writes `<home>/config.toml` with `[pdf] extractor = "<extractor>"` plus `extra` appended
    /// verbatim inside the same table (e.g. `"timeout_s = 1\nocr_min_chars = 20\n"`).
    pub fn config(&self, extractor: &Path, extra: &str) {
        let ext = extractor.to_string_lossy().replace('\\', "\\\\");
        fs::write(
            self.home.path().join("config.toml"),
            format!("[pdf]\nextractor = \"{ext}\"\n{extra}\n"),
        )
        .unwrap();
    }

    pub fn set_text(&self, mode: &str, pages: Option<&str>, text: &str) {
        let name = match pages {
            Some(p) => format!("{mode}-p{p}.txt"),
            None => format!("{mode}.txt"),
        };
        fs::write(self.control_dir().join(name), text).unwrap();
    }

    pub fn set_fail(&self, message: &str) {
        fs::write(self.control_dir().join("fail"), message).unwrap();
    }

    pub fn set_timeout(&self) {
        fs::write(self.control_dir().join("timeout"), "").unwrap();
    }

    pub fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.control_dir().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(|s| s.to_string())
            .collect()
    }
}

// Windows twin. Argument order it expects: `parse <pdf> --format text -o <out>
// [--no-ocr -q | --ocr-language <lang>] [--target-pages <range>]`, or `-V` alone. Must match
// `Liteparse::run` in `src/pdf.rs` (Task 2) — that task does not change this file; if the
// argument order ever needs to change, this template changes too, deliberately, as a fix round
// on THIS task, not a silent edit by whoever touches `pdf.rs` next.
const FAKE_LITEPARSE_CMD: &str = r#"@echo off
setlocal enabledelayedexpansion
set "CONTROL=__CONTROL_DIR__"
if "%~1"=="-V" (
  if exist "%CONTROL%\version.txt" (type "%CONTROL%\version.txt") else (echo liteparse 0.0.0-fake)
  exit /b 0
)
set NOOCR=0
set "OUT="
set "PAGES=-"
set NEXTOUT=0
set NEXTPAGES=0
:parseloop
if "%~1"=="" goto afterparse
if "%NEXTOUT%"=="1" (set "OUT=%~1" & set "NEXTOUT=0" & shift & goto parseloop)
if "%NEXTPAGES%"=="1" (set "PAGES=%~1" & set "NEXTPAGES=0" & shift & goto parseloop)
if "%~1"=="-o" (set "NEXTOUT=1" & shift & goto parseloop)
if "%~1"=="--target-pages" (set "NEXTPAGES=1" & shift & goto parseloop)
if "%~1"=="--no-ocr" (set "NOOCR=1" & shift & goto parseloop)
shift
goto parseloop
:afterparse
if "%NOOCR%"=="1" (set MODE=noocr) else (set MODE=ocr)
>>"%CONTROL%\calls.log" echo %MODE% pages=%PAGES%
if exist "%CONTROL%\timeout" (ping -n 5 127.0.0.1 >nul & exit /b 1)
if exist "%CONTROL%\fail" (type "%CONTROL%\fail" & exit /b 1)
set "SRC=%CONTROL%\%MODE%.txt"
if not "%PAGES%"=="-" if exist "%CONTROL%\%MODE%-p%PAGES%.txt" set "SRC=%CONTROL%\%MODE%-p%PAGES%.txt"
if exist "%SRC%" (copy /y "%SRC%" "%OUT%" >nul) else (type nul > "%OUT%")
echo [liteparse] extract: 5.0ms (7 pages)
exit /b 0
"#;

// POSIX twin: same contract, untested on this Windows-only machine, kept for group 5.
const FAKE_LITEPARSE_SH: &str = r#"#!/bin/sh
CONTROL="__CONTROL_DIR__"
if [ "$1" = "-V" ]; then
  if [ -f "$CONTROL/version.txt" ]; then cat "$CONTROL/version.txt"; else echo "liteparse 0.0.0-fake"; fi
  exit 0
fi
NOOCR=0
OUT=""
PAGES="-"
while [ $# -gt 0 ]; do
  case "$1" in
    -o) OUT="$2"; shift 2 ;;
    --target-pages) PAGES="$2"; shift 2 ;;
    --no-ocr) NOOCR=1; shift ;;
    *) shift ;;
  esac
done
if [ "$NOOCR" = "1" ]; then MODE=noocr; else MODE=ocr; fi
echo "$MODE pages=$PAGES" >> "$CONTROL/calls.log"
if [ -f "$CONTROL/timeout" ]; then sleep 5; exit 1; fi
if [ -f "$CONTROL/fail" ]; then cat "$CONTROL/fail"; exit 1; fi
SRC="$CONTROL/$MODE.txt"
if [ "$PAGES" != "-" ] && [ -f "$CONTROL/$MODE-p$PAGES.txt" ]; then SRC="$CONTROL/$MODE-p$PAGES.txt"; fi
if [ -f "$SRC" ]; then cp "$SRC" "$OUT"; else : > "$OUT"; fi
echo "[liteparse] extract: 5.0ms (7 pages)"
exit 0
"#;

/// Run `ratchet pdf <args>` against the sandbox. Every scenario test goes through this helper.
pub fn pdf(pb: &PdfBox, args: &[&str]) -> Output {
    Command::new(ratchet_bin())
        .arg("pdf")
        .args(args)
        .current_dir(pb.work.path())
        .env("RATCHET_HOME", pb.home.path())
        .env_remove("RATCHET_SESSION_ID")
        .output()
        .unwrap()
}

// --- group 2: the board ---------------------------------------------------------------------

/// A sandbox whose database exists and holds one session registered at `T0`. The session-start
/// hook is the only face allowed to create the database, so this is also how a test gets one.
pub fn board(session_id: &str) -> Sandbox {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload(session_id, &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "session-start failed: {}", stderr(&out));
    sb
}

/// Register one more session in the same sandbox, at `T0 + minutes`.
pub fn join(sb: &Sandbox, session_id: &str, minutes: i64) -> Output {
    let when = at(minutes);
    let root = sb.root();
    hook_env(
        sb,
        "session-start",
        &session_payload(session_id, &root),
        &root,
        &[("RATCHET_NOW", &when)],
    )
}

/// `ratchet task …` as `session_id`, at `T0 + minutes`, from the repo root.
pub fn task(sb: &Sandbox, args: &[&str], session_id: &str, minutes: i64) -> Output {
    let when = at(minutes);
    let mut argv = vec!["task"];
    argv.extend_from_slice(args);
    cli(
        sb,
        &argv,
        &sb.root(),
        &[("RATCHET_SESSION_ID", session_id), ("RATCHET_NOW", &when)],
    )
}

/// Create a task and return its identifier (the first token the command prints).
pub fn new_task(
    sb: &Sandbox,
    title: &str,
    checks: &[&str],
    session_id: &str,
    minutes: i64,
) -> String {
    let mut args = vec!["new", title];
    for c in checks {
        args.push("--check");
        args.push(c);
    }
    let out = task(sb, &args, session_id, minutes);
    assert_eq!(code(&out), 0, "task new failed: {}", stderr(&out));
    stdout(&out)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Kinds of a task's events, oldest first.
pub fn kinds_of(sb: &Sandbox, task_id: &str) -> Vec<String> {
    let conn = db(sb);
    let mut stmt = conn
        .prepare("SELECT kind FROM events WHERE task_id = ?1 ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![task_id], |r| r.get::<_, String>(0))
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

/// Raw payloads of a task's events of one kind, oldest first.
pub fn payloads_of(sb: &Sandbox, task_id: &str, kind: &str) -> Vec<String> {
    let conn = db(sb);
    let mut stmt = conn
        .prepare("SELECT payload FROM events WHERE task_id = ?1 AND kind = ?2 ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![task_id, kind], |r| r.get::<_, String>(0))
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

/// `(status, claimed_by, archived_at)` straight from the row.
pub fn task_state(sb: &Sandbox, task_id: &str) -> (String, Option<String>, Option<String>) {
    db(sb)
        .query_row(
            "SELECT status, claimed_by, archived_at FROM tasks WHERE id = ?1",
            rusqlite::params![task_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

/// Files the output discipline wrote under the state directory.
pub fn out_files(sb: &Sandbox) -> Vec<PathBuf> {
    let dir = sb.home.path().join("out");
    match fs::read_dir(&dir) {
        Err(_) => Vec::new(),
        Ok(entries) => entries.map(|e| e.unwrap().path()).collect(),
    }
}

/// Non-empty lines of a command's standard output.
pub fn lines(out: &Output) -> Vec<String> {
    stdout(out)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

// --- group 6: map ----------------------------------------------------------------------------

/// Run `ratchet map <args>` against the sandbox, from its root.
pub fn map(sb: &Sandbox, args: &[&str]) -> Output {
    Command::new(ratchet_bin())
        .arg("map")
        .args(args)
        .current_dir(sb.root())
        .env("RATCHET_HOME", sb.home.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap()
}

/// Like `map`, but with extra environment (`RATCHET_NOW`, mainly).
pub fn map_env(sb: &Sandbox, args: &[&str], extra: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.arg("map")
        .args(args)
        .current_dir(sb.root())
        .env("RATCHET_HOME", sb.home.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// Writes a `.rs` file with a `//!` doc-comment header (or none, when `header` is `None`) and a
/// short body — enough to be a plausible Rust module without needing real code.
pub fn write_rust_module(sb: &Sandbox, rel: &str, header: Option<&str>, body: &str) -> PathBuf {
    let text = match header {
        Some(h) => format!("//! {h}\n\n{body}\n"),
        None => format!("{body}\n"),
    };
    sb.write(rel, &text)
}

/// Writes a `.py` file with a triple-quoted module docstring (or none).
pub fn write_python_module(sb: &Sandbox, rel: &str, header: Option<&str>, body: &str) -> PathBuf {
    let text = match header {
        Some(h) => format!("\"\"\"{h}\"\"\"\n\n{body}\n"),
        None => format!("{body}\n"),
    };
    sb.write(rel, &text)
}

/// Writes a `.ts` file with a `/** … */` leading block comment (or none).
pub fn write_ts_module(sb: &Sandbox, rel: &str, header: Option<&str>, body: &str) -> PathBuf {
    let text = match header {
        Some(h) => format!("/**\n * {h}\n */\n\n{body}\n"),
        None => format!("{body}\n"),
    };
    sb.write(rel, &text)
}

/// Writes a file with no recognised header at all — any extension, any content.
pub fn write_plain_file(sb: &Sandbox, rel: &str, body: &str) -> PathBuf {
    sb.write(rel, body)
}

/// `git add <files> && git commit -m <msg>` in the sandbox's repo — the only way a fixture file
/// becomes visible to `git ls-files`, which is what `ratchet map` actually reads.
pub fn commit(sb: &Sandbox, files: &[&str], msg: &str) {
    let root = sb.repo.path();
    let mut args = vec!["add"];
    args.extend_from_slice(files);
    git(root, &args);
    git(root, &["commit", "-q", "-m", msg]);
}

/// `HEAD`'s full sha, from the sandbox's repo.
pub fn head_sha(sb: &Sandbox) -> String {
    let out = Command::new("git")
        .args(["-C", &sb.repo.path().to_string_lossy(), "rev-parse", "HEAD"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The written map's text, or empty when absent.
pub fn read_map(sb: &Sandbox) -> String {
    fs::read_to_string(sb.root().join(".ratchet/map.md")).unwrap_or_default()
}

/// `.ratchet/map.notes`'s text, or empty when absent.
pub fn read_notes(sb: &Sandbox) -> String {
    fs::read_to_string(sb.root().join(".ratchet/map.notes")).unwrap_or_default()
}

/// Replaces `CLAUDE.md` at the sandbox root with a symlink to `target` — used only by the
/// `--wire` refusal scenario. Unix-only (`cfg(unix)`), matching every other symlink-dependent
/// test convention in this repo (there are none yet outside this one; keep it gated the same way
/// the rest of the suite gates anything OS-specific).
#[cfg(unix)]
pub fn symlink_claude_md(sb: &Sandbox, target: &Path) {
    use std::os::unix::fs::symlink;
    let link = sb.root().join("CLAUDE.md");
    let _ = fs::remove_file(&link);
    symlink(target, &link).unwrap();
}

/// Run `ratchet map <args>` with no sandbox at all — for the "outside a marker repo" scenarios,
/// where `dir` deliberately has no `ratchet.toml` above it (`unmanaged_dir()`).
pub fn map_at(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(ratchet_bin())
        .arg("map")
        .args(args)
        .current_dir(dir)
        .env("RATCHET_HOME", home)
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap()
}
