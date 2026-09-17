//! `ratchet db migrate | path | selftest`.

use std::collections::HashMap;

use crate::config::ratchet_home;
use crate::db;

/// Applies pending migrations. One of the only two places allowed to migrate (spec §6).
pub fn migrate(env: &HashMap<String, String>) -> i32 {
    let home = ratchet_home(env);
    let mut conn = match db::open(&db::db_path(&home)) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    match db::migrate(&mut conn) {
        Err(e) => fail(e),
        Ok(applied) if applied.is_empty() => {
            println!("already at version {}", db::current_version(&conn));
            0
        }
        Ok(applied) => {
            for (version, name) in applied {
                println!("applied {version:04} {name}");
            }
            println!("now at version {}", db::current_version(&conn));
            0
        }
    }
}

pub fn path(env: &HashMap<String, String>) -> i32 {
    println!("{}", db::db_path(&ratchet_home(env)).display());
    0
}

pub fn selftest() -> i32 {
    match db::selftest() {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(e) => fail(e),
    }
}

fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}
