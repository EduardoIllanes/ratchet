//! One-line append to `<home>/ratchet.log`. Never fails: a hook must not die because the log did.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub fn append(home: &Path, line: &str) {
    let _ = fs::create_dir_all(home);
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("ratchet.log"))
    {
        let stamp = chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let _ = writeln!(f, "{stamp} {line}");
    }
}
