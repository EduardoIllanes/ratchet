use assert_cmd::Command;

#[test]
fn version_prints_name_and_semver() {
    let out = Command::cargo_bin("ratchet")
        .unwrap()
        .arg("version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let expected = format!("ratchet {}", env!("CARGO_PKG_VERSION"));
    assert!(text.starts_with(&expected), "got: {text}");
}
