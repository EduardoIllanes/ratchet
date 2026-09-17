//! Reports pre-tool latency and fails above the ceiling. Release numbers are what the README
//! quotes; run `cargo test --release --test latency -- --nocapture` to see them.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

#[test]
fn pre_tool_median_under_ceiling() {
    let home = tempfile::TempDir::new().unwrap();
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(repo.path().join(".venv")).unwrap();
    std::fs::write(repo.path().join("ratchet.toml"), "").unwrap();
    let payload = format!(
        r#"{{"tool_name":"Bash","tool_input":{{"command":"python x.py"}},"cwd":{}}}"#,
        serde_json::to_string(&repo.path().to_string_lossy()).unwrap()
    );
    let bin = assert_cmd::cargo::cargo_bin("ratchet");
    let mut times = Vec::new();
    for _ in 0..30 {
        let t = Instant::now();
        let mut child = Command::new(&bin)
            .args(["hook", "pre-tool"])
            .current_dir(repo.path())
            .env("RATCHET_HOME", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        let st = child.wait().unwrap();
        times.push(t.elapsed().as_micros());
        assert_eq!(st.code(), Some(2));
    }
    times.sort();
    let median_ms = times[times.len() / 2] as f64 / 1000.0;
    let p90_ms = times[times.len() * 9 / 10] as f64 / 1000.0;
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    eprintln!("pre-tool latency: median {median_ms:.1} ms, p90 {p90_ms:.1} ms ({build})");
    let ceiling = if cfg!(debug_assertions) { 200.0 } else { 60.0 };
    assert!(
        median_ms < ceiling,
        "median {median_ms:.1} ms over {ceiling} ms"
    );
}
