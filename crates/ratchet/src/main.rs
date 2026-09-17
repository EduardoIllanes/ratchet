//! ratchet — thin CLI face. All logic lives in the modules; main only routes.

mod cli;
mod clock;
mod config;
mod db;
mod guardrails;
mod hooks;
mod log;
mod model;
mod output;
mod pdf;
mod repo;
mod services;

use std::collections::HashMap;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ratchet",
    version,
    about = "Guardrailed, spec-driven harness for Claude Code"
)]
struct Cli {
    /// Session to attribute writes to. Overrides `RATCHET_SESSION_ID` and the resolution by
    /// directory, and is accepted before or after the subcommand.
    #[arg(long, global = true)]
    session: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a Claude Code hook. Reads the JSON payload on stdin.
    Hook { event: String },
    /// Inspect or dry-run the guardrails that apply in the current directory.
    Guardrails {
        #[command(subcommand)]
        cmd: GuardrailsCmd,
    },
    /// State database: `migrate`, `path`. `selftest` proves the embedded SQLite works.
    Db {
        #[command(subcommand)]
        cmd: DbCmd,
    },
    /// Print the version.
    Version,
    /// Agent sessions: `list`, `show`.
    Session {
        #[command(subcommand)]
        cmd: SessionCmd,
    },
    /// The task board: `list`, `show`, `new`, `claim`, `status`, `check`, `note`, `handoff`,
    /// `archive`, `unarchive`.
    Task {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
    /// Extract text from a local PDF via the external `liteparse` CLI.
    Pdf {
        /// Path to the PDF file.
        file: std::path::PathBuf,
        /// Page range, e.g. "1-8,12". Narrows the extraction and the sink file name.
        #[arg(long)]
        pages: Option<String>,
        /// Force OCR from the start; skips the fast pass.
        #[arg(long)]
        ocr: bool,
    },
}

#[derive(Subcommand)]
enum GuardrailsCmd {
    /// List the active rules (built-in, machine and repo), marking disabled ones.
    List,
    /// Evaluate one tool call: `ratchet guardrails test Bash '{"command":"python x.py"}'`.
    /// Exit 2 when a rule blocks, 0 otherwise.
    Test { tool: String, payload: String },
}

#[derive(Subcommand)]
enum DbCmd {
    /// Apply pending migrations to the state database.
    Migrate,
    /// Print the path of the state database.
    Path,
    /// Print one line proving the bundled SQLite is linked and executes SQL.
    Selftest,
}

#[derive(Subcommand)]
enum SessionCmd {
    /// One line per session with its derived state.
    List {
        /// Only sessions of this repo (the name in the marker, or the directory name).
        #[arg(long)]
        repo: Option<String>,
        /// Only the live ones.
        #[arg(long)]
        live: bool,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Detail of one session; without an id, the session covering this directory.
    Show {
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TaskCmd {
    /// One line per task: id, status, priority, title and progress.
    List {
        /// Tasks of the repo with this name, across the machine.
        #[arg(long)]
        repo: Option<String>,
        /// Only these statuses; repeatable.
        #[arg(long = "status", short = 's')]
        statuses: Vec<String>,
        /// Only the ones this session holds.
        #[arg(long)]
        mine: bool,
        /// Only the ones carrying this tag.
        #[arg(long)]
        tag: Option<String>,
        /// Include archived tasks.
        #[arg(long, short = 'a')]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Body, checklist, last handoff and the last ten events of one task.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Create a task in this repo, in `backlog`.
    New {
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long = "body-file")]
        body_file: Option<std::path::PathBuf>,
        /// One acceptance criterion; repeatable, in order.
        #[arg(long = "check", short = 'c')]
        checks: Vec<String>,
        #[arg(long, short = 'p', default_value_t = 3)]
        priority: i64,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Take a task for this session.
    Claim {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Move a task: backlog, ready, in_progress, blocked, review, done.
    Status {
        id: String,
        to: String,
        /// Reason; required to close a task that has no checklist.
        #[arg(long)]
        why: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Mark a checklist item as done, or undo it.
    Check {
        id: String,
        position: i64,
        #[arg(long)]
        undo: bool,
        #[arg(long)]
        json: bool,
    },
    /// Record a decision or a finding on a task.
    Note {
        id: String,
        text: String,
        #[arg(long)]
        json: bool,
    },
    /// Record what is left and how to resume.
    Handoff {
        id: String,
        text: String,
        #[arg(long)]
        json: bool,
    },
    /// Hide a done task from the board.
    Archive {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Bring an archived task back.
    Unarchive {
        id: String,
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => match e.kind() {
            // Not a true error: `--help`/`--version` print to stdout and exit 0. `e.exit()`
            // never returns. Bare `ratchet` (no subcommand) is NOT included here: clap renders
            // that case's help through `Error::stream()` to stderr with its usage exit code (2)
            // even via `e.exit()`, so it falls into the catch-all below instead, matching every
            // other usage error.
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => e.exit(),
            // Every other clap error (unknown subcommand, a non-numeric value for a typed
            // argument, a missing required argument, …) is a user-facing CLI error: print
            // clap's own formatted message (it already starts with `error:`) and exit 1, not
            // clap's default 2, which this binary reserves for a guardrail block (spec D-p3).
            _ => {
                let _ = e.print();
                std::process::exit(1);
            }
        },
    };
    let env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().ok();
    let session = cli.session.clone();
    let code = match cli.cmd {
        Cmd::Version => {
            println!("ratchet {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Cmd::Hook { event } => hooks::run(&event, std::io::stdin().lock(), &env, cwd),
        Cmd::Guardrails { cmd } => match cmd {
            GuardrailsCmd::List => guardrails::cli::list(&env, cwd),
            GuardrailsCmd::Test { tool, payload } => {
                guardrails::cli::test(&tool, &payload, &env, cwd)
            }
        },
        Cmd::Db { cmd } => match cmd {
            DbCmd::Migrate => cli::db_cmd::migrate(&env),
            DbCmd::Path => cli::db_cmd::path(&env),
            DbCmd::Selftest => cli::db_cmd::selftest(),
        },
        Cmd::Session { cmd } => match cmd {
            SessionCmd::List { repo, live, json } => {
                cli::session_cmd::list(&env, cwd, repo.as_deref(), live, json)
            }
            SessionCmd::Show { id, json } => {
                cli::session_cmd::show(&env, cwd, id.as_deref().or(session.as_deref()), json)
            }
        },
        Cmd::Task { cmd } => match cmd {
            TaskCmd::List {
                repo,
                statuses,
                mine,
                tag,
                all,
                json,
            } => cli::task_cmd::list(
                &env,
                cwd,
                session.as_deref(),
                repo.as_deref(),
                &statuses,
                mine,
                tag.as_deref(),
                all,
                json,
            ),
            TaskCmd::Show { id, json } => cli::task_cmd::show(&env, cwd, &id, json),
            TaskCmd::New {
                title,
                body,
                body_file,
                checks,
                priority,
                tags,
                parent,
                json,
            } => cli::task_cmd::new(
                &env,
                cwd,
                session.as_deref(),
                &title,
                body.as_deref(),
                body_file,
                &checks,
                priority,
                &tags,
                parent.as_deref(),
                json,
            ),
            TaskCmd::Claim { id, json } => {
                cli::task_cmd::claim(&env, cwd, session.as_deref(), &id, json)
            }
            TaskCmd::Status { id, to, why, json } => cli::task_cmd::status(
                &env,
                cwd,
                session.as_deref(),
                &id,
                &to,
                why.as_deref(),
                json,
            ),
            TaskCmd::Check {
                id,
                position,
                undo,
                json,
            } => cli::task_cmd::check(&env, cwd, session.as_deref(), &id, position, undo, json),
            TaskCmd::Note { id, text, json } => {
                cli::task_cmd::note(&env, cwd, session.as_deref(), &id, &text, json)
            }
            TaskCmd::Handoff { id, text, json } => {
                cli::task_cmd::handoff(&env, cwd, session.as_deref(), &id, &text, json)
            }
            TaskCmd::Archive { id, json } => {
                cli::task_cmd::archive(&env, cwd, session.as_deref(), &id, json)
            }
            TaskCmd::Unarchive { id, json } => {
                cli::task_cmd::unarchive(&env, cwd, session.as_deref(), &id, json)
            }
        },
        Cmd::Pdf { file, pages, ocr } => cli::pdf_cmd::run(&file, pages.as_deref(), ocr, &env),
    };
    std::process::exit(code);
}
