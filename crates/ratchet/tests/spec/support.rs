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

/// Turns a `bash`/`powershell` pre-tool payload into its matching PostToolUse payload: same
/// `tool_name`, `tool_input` and `cwd`, plus an empty `tool_response`.
pub fn post_tool(payload: &Value) -> Value {
    let mut v = payload.clone();
    v.as_object_mut()
        .unwrap()
        .insert("tool_response".to_string(), json!({}));
    v
}

/// A `Read` call with no window (no `offset`, no `limit`).
pub fn read(file_path: &Path, cwd: &Path) -> Value {
    json!({ "tool_name": "Read", "tool_input": { "file_path": file_path.to_string_lossy() }, "cwd": cwd.to_string_lossy() })
}

/// A `Read` call with an explicit window. Either bound may be omitted (`None`); the
/// corresponding key is left out of `tool_input` rather than sent as `null`, matching what
/// Claude Code itself sends.
pub fn read_window(file_path: &Path, offset: Option<i64>, limit: Option<i64>, cwd: &Path) -> Value {
    let mut input = serde_json::Map::new();
    input.insert("file_path".to_string(), json!(file_path.to_string_lossy()));
    if let Some(o) = offset {
        input.insert("offset".to_string(), json!(o));
    }
    if let Some(l) = limit {
        input.insert("limit".to_string(), json!(l));
    }
    json!({ "tool_name": "Read", "tool_input": Value::Object(input), "cwd": cwd.to_string_lossy() })
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

/// Adds `rel` as a newly tracked (committed) file in `sb`'s main tree, so a test can exercise a
/// guardrail shape against a tracked path other than the fixture's default `tracked.txt`.
pub fn track_file(sb: &Sandbox, rel: &str, body: &str) -> PathBuf {
    let p = sb.write(rel, body);
    let root = sb.root();
    git(&root, &["add", rel]);
    git(&root, &["commit", "-q", "-m", &format!("track {rel}")]);
    p
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

/// Raw payloads of a session's events of one kind, oldest first (mirrors `payloads_of`, which
/// is keyed by task instead of session — used for events, like a guardrail record, that belong
/// to a session rather than a task).
pub fn session_event_payloads(sb: &Sandbox, session_id: &str, kind: &str) -> Vec<String> {
    let conn = db(sb);
    let mut stmt = conn
        .prepare("SELECT payload FROM events WHERE session_id = ?1 AND kind = ?2 ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![session_id, kind], |r| {
            r.get::<_, String>(0)
        })
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
#[allow(dead_code)] // kept for later map tasks; no scenario calls it yet
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

// --- group 7: usage --------------------------------------------------------------------------
//
// `TranscriptBuilder` writes JSONL transcripts under a temp "projects" root, matching exactly
// the fields design §2 (`docs/superpowers/specs/2026-09-21-ratchet-usage-design.md`) verified on
// real Claude Code output: `type`, `timestamp`, `sessionId`, `cwd`, `gitBranch`, `version`,
// `message.model`, `message.usage.{input_tokens,cache_creation_input_tokens,
// cache_read_input_tokens,output_tokens,output_tokens_details.thinking_tokens}`,
// `message.content` holding `tool_use` blocks with `id`/`name`. Point `RATCHET_CLAUDE_PROJECTS`
// at `.root` (see `usage()` below). Every record uses a fixed `version` ("1.2.3") and
// `gitBranch` ("main") — no scenario in this plan needs either to vary; a scenario that does can
// still reach into `.root` directly and write its own record.

/// Four token classes plus optional thinking, in the units the fixture's callers already think
/// in (plain `u64`, not yet abbreviated — that only happens on the way to a terminal).
pub struct Usage {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub thinking: Option<u64>,
}

// NOTE (Task 1 tension, flagged for the owner): the brief's verbatim code names both this
// token-tuple builder and the `ratchet usage` process runner below `usage`, which cannot coexist
// in one module (E0428, no overloading in Rust). Renamed this one -- the smaller, purely-local
// convenience -- to `tokens`, keeping `usage(sb, tb, args, cwd, extra)` as `usage` since it is
// the helper named in its own doc comment ("Run `ratchet usage <args>` ... every `usage__*`
// scenario test goes through this helper") and matches the subcommand name it drives.
pub fn tokens(input: u64, cache_write: u64, cache_read: u64, output: u64) -> Usage {
    Usage {
        input,
        cache_write,
        cache_read,
        output,
        thinking: None,
    }
}

pub struct TranscriptBuilder {
    pub root: TempDir,
}

impl TranscriptBuilder {
    pub fn new() -> Self {
        TranscriptBuilder {
            root: TempDir::new().unwrap(),
        }
    }

    fn slug(cwd: &Path) -> String {
        cwd.to_string_lossy().replace(['/', '\\'], "-")
    }

    fn transcript_path(&self, cwd: &Path, session_id: &str) -> PathBuf {
        self.root
            .path()
            .join(Self::slug(cwd))
            .join(format!("{session_id}.jsonl"))
    }

    fn subagents_dir(&self, cwd: &Path, session_id: &str) -> PathBuf {
        self.root
            .path()
            .join(Self::slug(cwd))
            .join(session_id)
            .join("subagents")
    }

    fn append(path: &Path, record: &Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f, "{record}").unwrap();
    }

    fn append_raw(path: &Path, line: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f, "{line}").unwrap();
    }

    fn record(
        ts: &str,
        session_id: &str,
        cwd: &Path,
        model: &str,
        u: &Usage,
        content: Value,
    ) -> Value {
        let mut message = json!({ "model": model, "content": content });
        message["usage"] = json!({
            "input_tokens": u.input,
            "cache_creation_input_tokens": u.cache_write,
            "cache_read_input_tokens": u.cache_read,
            "output_tokens": u.output,
        });
        if let Some(t) = u.thinking {
            message["usage"]["output_tokens_details"] = json!({ "thinking_tokens": t });
        }
        json!({
            "type": "assistant",
            "timestamp": ts,
            "sessionId": session_id,
            "cwd": cwd.to_string_lossy(),
            "gitBranch": "main",
            "version": "1.2.3",
            "message": message,
        })
    }

    /// One assistant call, no `tool_use` blocks.
    pub fn call(&self, cwd: &Path, session_id: &str, ts: &str, model: &str, u: &Usage) -> &Self {
        let rec = Self::record(ts, session_id, cwd, model, u, json!([]));
        Self::append(&self.transcript_path(cwd, session_id), &rec);
        self
    }

    /// One assistant call whose content includes one `tool_use` block — used for an `Agent`
    /// dispatch (ends orientation, and its `id` is what a subagent's `toolUseId` matches) and for
    /// `Edit`/`Write`/`NotebookEdit` (also ends orientation, no `toolUseId` match needed).
    #[allow(clippy::too_many_arguments)] // fixture builder, one field per transcript column
    pub fn call_with_tool(
        &self,
        cwd: &Path,
        session_id: &str,
        ts: &str,
        model: &str,
        u: &Usage,
        tool_name: &str,
        tool_use_id: &str,
    ) -> &Self {
        let content = json!([{ "type": "tool_use", "id": tool_use_id, "name": tool_name }]);
        let rec = Self::record(ts, session_id, cwd, model, u, content);
        Self::append(&self.transcript_path(cwd, session_id), &rec);
        self
    }

    /// An assistant record with no `message.usage` key at all — R2's "partial" case.
    pub fn call_no_usage(&self, cwd: &Path, session_id: &str, ts: &str, model: &str) -> &Self {
        let rec = json!({
            "type": "assistant",
            "timestamp": ts,
            "sessionId": session_id,
            "cwd": cwd.to_string_lossy(),
            "gitBranch": "main",
            "version": "1.2.3",
            "message": { "model": model, "content": [] },
        });
        Self::append(&self.transcript_path(cwd, session_id), &rec);
        self
    }

    /// A line that is not valid JSON — R2's "skipped" case.
    pub fn garbage(&self, cwd: &Path, session_id: &str) -> &Self {
        Self::append_raw(
            &self.transcript_path(cwd, session_id),
            "not json at all {{{",
        );
        self
    }

    /// A well-formed record whose `type` is not `"assistant"` — ignored, never counted anywhere.
    pub fn non_assistant(&self, cwd: &Path, session_id: &str) -> &Self {
        let rec = json!({ "type": "user", "timestamp": "2026-01-01T00:00:00Z" });
        Self::append(&self.transcript_path(cwd, session_id), &rec);
        self
    }

    /// One call in a subagent's own transcript,
    /// `<slug>/<session_id>/subagents/agent-<agent_id>.jsonl`.
    pub fn subagent(
        &self,
        cwd: &Path,
        session_id: &str,
        agent_id: &str,
        ts: &str,
        model: &str,
        u: &Usage,
    ) -> &Self {
        let rec = Self::record(ts, session_id, cwd, model, u, json!([]));
        let path = self
            .subagents_dir(cwd, session_id)
            .join(format!("agent-{agent_id}.jsonl"));
        Self::append(&path, &rec);
        self
    }

    /// `agent-<agent_id>.meta.json` next to that subagent transcript.
    #[allow(clippy::too_many_arguments)] // fixture builder, one field per meta-file column
    pub fn meta(
        &self,
        cwd: &Path,
        session_id: &str,
        agent_id: &str,
        agent_type: &str,
        description: &str,
        model: &str,
        tool_use_id: Option<&str>,
    ) -> &Self {
        let mut m = json!({ "agentType": agent_type, "description": description, "model": model });
        if let Some(id) = tool_use_id {
            m["toolUseId"] = json!(id);
        }
        let path = self
            .subagents_dir(cwd, session_id)
            .join(format!("agent-{agent_id}.meta.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, m.to_string()).unwrap();
        self
    }
}

/// Run `ratchet usage <args>` against the sandbox, with `RATCHET_CLAUDE_PROJECTS` pointed at the
/// fixture builder's root. Every `usage__*` scenario test goes through this helper.
pub fn usage(
    sb: &Sandbox,
    tb: &TranscriptBuilder,
    args: &[&str],
    cwd: &Path,
    extra: &[(&str, &str)],
) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.arg("usage")
        .args(args)
        .current_dir(cwd)
        .env("RATCHET_HOME", sb.home.path())
        .env("RATCHET_CLAUDE_PROJECTS", tb.root.path())
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

/// Inserts a `subagent.start`/`subagent.stop` row directly, bypassing the CLI — the exact shape
/// group T-0006 ships (see Assumption 1): the row's `task_id` column carries the task; the
/// payload carries `agent_id`, `agent_type`, `description` only.
#[allow(clippy::too_many_arguments)] // fixture builder, one field per event-row column
pub fn seed_subagent_event(
    sb: &Sandbox,
    kind: &str,
    session_id: &str,
    agent_id: &str,
    agent_type: &str,
    description: &str,
    task_id: &str,
    ts: &str,
) {
    let conn = db(sb);
    conn.execute(
        "INSERT INTO events(ts, session_id, task_id, kind, payload, source) VALUES (?1,?2,?3,?4,?5,'hook')",
        rusqlite::params![
            ts,
            session_id,
            task_id,
            kind,
            json!({
                "agent_id": agent_id,
                "agent_type": agent_type,
                "description": description,
            })
            .to_string(),
        ],
    )
    .unwrap();
}

/// True if `text` contains `n` as a standalone number: the byte immediately before and after
/// every match is not an ASCII digit, `.` or `,` — so `contains_number(text, "40")` cannot be
/// satisfied by "140", "40.5", "1,400" or "2400". Every scenario that checks a specific total
/// uses this instead of `str::contains`, so an aggregate that is merely a superstring of the
/// right digits does not pass (fix round 1, finding 1/4).
pub fn contains_number(text: &str, n: &str) -> bool {
    let bytes = text.as_bytes();
    let mut start = 0usize;
    while let Some(rel) = text[start..].find(n) {
        let idx = start + rel;
        let before_ok = idx == 0 || !matches!(bytes[idx - 1], b'0'..=b'9' | b'.' | b',');
        let end = idx + n.len();
        let after_ok = end >= bytes.len() || !matches!(bytes[end], b'0'..=b'9' | b'.' | b',');
        if before_ok && after_ok {
            return true;
        }
        start = idx + 1;
    }
    false
}
