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
    assert!(text.starts_with("ratchet 0.1.0"), "got: {text}");
}
