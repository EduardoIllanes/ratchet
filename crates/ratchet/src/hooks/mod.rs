//! Entry point of every hook. Principle: a hook never breaks a session. Any error or panic is
//! exit 0 plus one log line; the only non-zero exit is a deliberate block (2).

pub mod briefing;
pub mod dispatch;
pub mod handoff_rule;

use std::collections::HashMap;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use crate::config::ratchet_home;
use crate::log;

pub const BLOCK: i32 = 2;

pub fn run(
    event: &str,
    mut stdin: impl Read,
    env: &HashMap<String, String>,
    process_cwd: Option<PathBuf>,
) -> i32 {
    // A panic must not print a backtrace into the session.
    std::panic::set_hook(Box::new(|_| {}));
    let home = ratchet_home(env);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut text = String::new();
        stdin
            .read_to_string(&mut text)
            .map_err(|e| format!("stdin: {e}"))?;
        let payload = dispatch::parse_payload(&text)?;
        dispatch::dispatch(event, payload, env, process_cwd, &home)
    }));
    match result {
        Ok(Ok(code)) => code,
        Ok(Err(msg)) => {
            log::append(&home, &format!("hook {event}: {msg}"));
            0
        }
        Err(_) => {
            log::append(&home, &format!("hook {event}: panic"));
            0
        }
    }
}
