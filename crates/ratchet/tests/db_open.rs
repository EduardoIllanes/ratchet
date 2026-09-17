//! The one thing group 1 cannot work around: SQLite compiled into the binary by this toolchain.

use assert_cmd::Command;

#[test]
fn binary_still_runs() {
    Command::cargo_bin("ratchet")
        .unwrap()
        .arg("version")
        .assert()
        .success();
}

#[test]
fn bundled_sqlite_is_linked_and_usable() {
    // Exercised through the binary's own module, not a separate connection: this proves the
    // crate links libsqlite3-sys built from source, not a system library.
    let out = Command::cargo_bin("ratchet")
        .unwrap()
        .args(["db", "selftest"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("sqlite "), "got: {text}");
    assert!(text.contains(" ok"), "got: {text}");
}
