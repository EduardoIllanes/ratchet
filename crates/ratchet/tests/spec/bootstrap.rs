//! Scenario tests for `openspec/specs/bootstrap/spec.md`. Every `#### Scenario` there has
//! exactly one test here, named by slug. Every test drives the real wrapper
//! (`hooks/run-hook.cmd`) as a subprocess, exactly as Claude Code would, and points
//! `RATCHET_RELEASE_BASE` at a local `file://` directory holding a fake release built from the
//! real test binary (`support::ratchet_bin()`) — no test touches the network.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

use crate::support::{code, ratchet_bin, stderr, stdout};

/// The binary name the current host's bootstrap.sh would install.
fn bin_name() -> &'static str {
    if cfg!(windows) {
        "ratchet.exe"
    } else {
        "ratchet"
    }
}

/// The archive extension the current host's bootstrap.sh would download.
fn ext_name() -> &'static str {
    if cfg!(windows) {
        "zip"
    } else {
        "tar.gz"
    }
}

/// The release target triple for the machine actually running the tests.
fn host_target() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else if cfg!(target_os = "linux") {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(target_os = "windows") {
        "x86_64-pc-windows-msvc"
    } else {
        panic!("unsupported test host")
    }
}

fn stamp_path(root: &Path) -> PathBuf {
    root.join("bin").join(".bootstrap-failed")
}

fn installed_bin_path(root: &Path) -> PathBuf {
    root.join("bin").join(bin_name())
}

/// A fresh plugin directory: `.claude-plugin/plugin.json` at `version`, the real
/// `hooks/run-hook.cmd` and `hooks/bootstrap.sh` copied from the repository, and an empty
/// `bin/`.
fn fake_plugin(version: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    fs::write(
        root.join(".claude-plugin/plugin.json"),
        format!("{{\"name\":\"ratchet\",\"version\":\"{version}\"}}"),
    )
    .unwrap();
    fs::create_dir_all(root.join("hooks")).unwrap();
    let repo_hooks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hooks");
    fs::copy(
        repo_hooks.join("run-hook.cmd"),
        root.join("hooks/run-hook.cmd"),
    )
    .unwrap();
    fs::copy(
        repo_hooks.join("bootstrap.sh"),
        root.join("hooks/bootstrap.sh"),
    )
    .unwrap();
    fs::create_dir_all(root.join("bin")).unwrap();
    dir
}

/// The sha256 of `path`, lowercase hex, computed by shelling out (no hashing crate is a
/// dependency of this workspace).
fn sha256_file(path: &Path) -> String {
    if cfg!(windows) {
        let out = Command::new("certutil")
            .arg("-hashfile")
            .arg(path)
            .arg("SHA256")
            .output()
            .expect("certutil available");
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .nth(1)
            .expect("certutil prints the hash on its second line")
            .trim()
            .replace(' ', "")
            .to_lowercase()
    } else {
        let via_shasum = Command::new("shasum")
            .arg("-a")
            .arg("256")
            .arg(path)
            .output();
        let out = match via_shasum {
            Ok(o) if o.status.success() => o,
            _ => Command::new("sha256sum")
                .arg(path)
                .output()
                .expect("shasum or sha256sum available"),
        };
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .expect("hash tool prints the hash first")
            .to_lowercase()
    }
}

/// The release asset's file name for `version`/`target` on the current host.
fn asset_name(version: &str, target: &str) -> String {
    format!("ratchet-{version}-{target}.{}", ext_name())
}

/// Builds `ratchet-<version>-<target>.<ext>` inside `dir` (containing only the real test binary,
/// renamed to `bin_name()`), without writing `SHA256SUMS.txt`. Returns the asset's file name.
fn build_asset(dir: &Path, version: &str, target: &str) -> String {
    fs::create_dir_all(dir).unwrap();
    let asset = asset_name(version, target);
    let asset_path = dir.join(&asset);

    let stage = TempDir::new().unwrap();
    let staged_bin = stage.path().join(bin_name());
    fs::copy(ratchet_bin(), &staged_bin).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&staged_bin).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&staged_bin, perm).unwrap();
    }

    if cfg!(windows) {
        let status = Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg(format!(
                "Compress-Archive -Path '{}' -DestinationPath '{}' -Force",
                staged_bin.display(),
                asset_path.display()
            ))
            .status()
            .expect("powershell available");
        assert!(status.success(), "Compress-Archive failed");
    } else {
        let status = Command::new("tar")
            .arg("-C")
            .arg(stage.path())
            .arg("-czf")
            .arg(&asset_path)
            .arg(bin_name())
            .status()
            .expect("tar available");
        assert!(status.success(), "tar -czf failed");
    }

    asset
}

/// Builds the release asset plus `SHA256SUMS.txt` inside `dir`. `good_hash = false` writes 64
/// zeros for the asset's checksum instead of the real one, so the wrapper refuses it.
fn fake_release(dir: &Path, version: &str, target: &str, good_hash: bool) {
    let asset = build_asset(dir, version, target);
    let asset_path = dir.join(&asset);
    let real_hash = sha256_file(&asset_path);
    let hash = if good_hash { real_hash } else { "0".repeat(64) };
    fs::write(dir.join("SHA256SUMS.txt"), format!("{hash}  {asset}\n")).unwrap();
}

/// `file://` form of a local directory, as `RATCHET_RELEASE_BASE` expects it.
fn file_url(dir: &Path) -> String {
    if cfg!(windows) {
        format!("file:///{}", dir.display().to_string().replace('\\', "/"))
    } else {
        format!("file://{}", dir.display())
    }
}

/// Sets a file's mtime to well over 60 minutes in the past, so the bootstrap stamp reads as
/// stale.
fn age_stamp(path: &Path) {
    if cfg!(windows) {
        let status = Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg(format!(
                "(Get-Item '{}').LastWriteTime = (Get-Date).AddMinutes(-120)",
                path.display()
            ))
            .status()
            .expect("powershell available");
        assert!(status.success(), "aging the stamp failed");
    } else {
        // A fixed, far-past timestamp is trivially older than 60 minutes on any clock.
        let status = Command::new("touch")
            .arg("-t")
            .arg("202001010000")
            .arg(path)
            .status()
            .expect("touch available");
        assert!(status.success(), "aging the stamp failed");
    }
}

/// Runs the real wrapper as Claude Code would: `bash <plugin>/hooks/run-hook.cmd <args>`, with
/// `RATCHET_BIN` cleared so a developer's own environment never leaks into the test, and stdin
/// closed (these tests call CLI subcommands like `version`, not a hook event, so nothing needs
/// to be piped in).
fn run_wrapper(root: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("bash");
    cmd.arg(root.join("hooks/run-hook.cmd"))
        .args(args)
        .env_remove("RATCHET_BIN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// Runs `hooks/bootstrap.sh` directly (not through the wrapper), to check its own stderr
/// contract in isolation: exactly one line on success, exactly one on failure (or none, while
/// the stamp is young). The wrapper's combined stderr additionally carries its own mandated
/// "binary not found ... see the line above, if any" line whenever nothing ends up installed,
/// so "exactly one line" is only ever true of bootstrap.sh's own output, not the wrapper's.
fn run_bootstrap(root: &Path, envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("bash");
    cmd.arg(root.join("hooks/bootstrap.sh"))
        .arg(root)
        .env_remove("RATCHET_BIN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

#[test]
fn bootstrap__first_run_downloads_verifies_and_runs_the_binary() {
    let plugin = fake_plugin("0.1.0");
    let root = plugin.path();
    let release = TempDir::new().unwrap();
    fake_release(release.path(), "0.1.0", host_target(), true);

    let out = run_wrapper(
        root,
        &["version"],
        &[("RATCHET_RELEASE_BASE", &file_url(release.path()))],
    );

    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "ratchet 0.1.0");
    assert!(installed_bin_path(root).exists());
    let err = stderr(&out);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 1, "expected exactly one stderr line: {err}");
    assert!(lines[0].contains("installed"), "{err}");
    assert!(!stamp_path(root).exists());
}

#[test]
fn bootstrap__checksum_mismatch_refuses_the_download() {
    let plugin = fake_plugin("0.1.0");
    let root = plugin.path();
    let release = TempDir::new().unwrap();
    fake_release(release.path(), "0.1.0", host_target(), false);

    let out = run_wrapper(
        root,
        &["version"],
        &[("RATCHET_RELEASE_BASE", &file_url(release.path()))],
    );

    assert_eq!(code(&out), 0);
    let err = stderr(&out);
    assert!(err.contains("checksum mismatch"), "{err}");
    assert!(!installed_bin_path(root).exists());
    assert!(stamp_path(root).exists());
    fs::remove_file(stamp_path(root)).unwrap();

    // bootstrap.sh's own stderr contract ("on failure exactly one line") checked directly.
    let direct = run_bootstrap(root, &[("RATCHET_RELEASE_BASE", &file_url(release.path()))]);
    assert_eq!(code(&direct), 1);
    let derr = stderr(&direct);
    assert_eq!(
        derr.lines().count(),
        1,
        "expected exactly one stderr line from bootstrap.sh itself: {derr}"
    );
    assert!(derr.contains("checksum mismatch"), "{derr}");
}

#[test]
fn bootstrap__a_failed_bootstrap_is_silent_until_the_stamp_expires() {
    let plugin = fake_plugin("0.1.0");
    let root = plugin.path();
    // An existing, empty directory: the asset is never there, so every download fails.
    let empty_release = TempDir::new().unwrap();

    let first = run_wrapper(
        root,
        &["version"],
        &[("RATCHET_RELEASE_BASE", &file_url(empty_release.path()))],
    );
    assert_eq!(code(&first), 0, "stderr: {}", stderr(&first));
    assert!(stamp_path(root).exists());
    assert!(!installed_bin_path(root).exists());

    let second = run_wrapper(
        root,
        &["version"],
        &[("RATCHET_RELEASE_BASE", &file_url(empty_release.path()))],
    );
    assert_eq!(code(&second), 0);
    let err = stderr(&second);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly one stderr line while the stamp is young: {err}"
    );
    assert!(
        lines[0].contains("Bootstrap did not install it"),
        "expected the wrapper's own not-found line, got: {err}"
    );
    assert!(!installed_bin_path(root).exists());

    age_stamp(&stamp_path(root));

    let release = TempDir::new().unwrap();
    fake_release(release.path(), "0.1.0", host_target(), true);
    let third = run_wrapper(
        root,
        &["version"],
        &[("RATCHET_RELEASE_BASE", &file_url(release.path()))],
    );
    assert_eq!(code(&third), 0, "stderr: {}", stderr(&third));
    assert_eq!(stdout(&third).trim(), "ratchet 0.1.0");
    assert!(installed_bin_path(root).exists());
    assert!(!stamp_path(root).exists());
}

#[test]
fn bootstrap__unsupported_platform_is_reported_once() {
    let plugin = fake_plugin("0.1.0");
    let root = plugin.path();
    // Never reached (target resolution fails first), but set anyway so no test can ever
    // accidentally reach the real network.
    let release = TempDir::new().unwrap();

    let out = run_wrapper(
        root,
        &["version"],
        &[
            ("RATCHET_OS", "Plan9"),
            ("RATCHET_RELEASE_BASE", &file_url(release.path())),
        ],
    );

    assert_eq!(code(&out), 0);
    let err = stderr(&out);
    assert!(err.contains("unsupported platform"), "{err}");
    assert!(stamp_path(root).exists());
    assert!(!installed_bin_path(root).exists());

    // A second run within 60 minutes must be silenced by the stamp, exactly like any other
    // failure kind: the unsupported-platform check must never outrun the stamp check.
    let second = run_wrapper(
        root,
        &["version"],
        &[
            ("RATCHET_OS", "Plan9"),
            ("RATCHET_RELEASE_BASE", &file_url(release.path())),
        ],
    );
    assert_eq!(code(&second), 0);
    let err2 = stderr(&second);
    let lines: Vec<&str> = err2.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly one stderr line on the second run: {err2}"
    );
    assert!(
        lines[0].contains("Bootstrap did not install it"),
        "expected the wrapper's own not-found line, got: {err2}"
    );
    assert!(
        !err2.contains("[ratchet] bootstrap failed"),
        "no bootstrap line expected while the stamp is young: {err2}"
    );
}

#[test]
fn bootstrap__an_existing_binary_is_never_re_downloaded() {
    let plugin = fake_plugin("0.1.0");
    let root = plugin.path();
    let bin_path = installed_bin_path(root);
    fs::copy(ratchet_bin(), &bin_path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&bin_path).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&bin_path, perm).unwrap();
    }

    let missing_dir = root.join("no-such-release-dir");

    let out = run_wrapper(
        root,
        &["version"],
        &[("RATCHET_RELEASE_BASE", &file_url(&missing_dir))],
    );

    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "ratchet 0.1.0");
    assert!(!stamp_path(root).exists());
}

#[test]
fn bootstrap__download_failure_names_the_manual_path() {
    let plugin = fake_plugin("0.1.0");
    let root = plugin.path();

    // Run 1: an existing, empty directory — no asset, no sums. The scenario's literal WHEN
    // ("a directory without the asset"). After the I1a reorder this fails on SHA256SUMS.txt
    // first, so the assertion stays generic ("download failed", not naming a specific file).
    let empty_release = TempDir::new().unwrap();
    let empty_base = file_url(empty_release.path());
    let out = run_wrapper(root, &["version"], &[("RATCHET_RELEASE_BASE", &empty_base)]);
    assert_eq!(code(&out), 0);
    let err = stderr(&out);
    assert!(err.contains("download failed"), "{err}");
    assert!(err.contains("RATCHET_BIN"), "{err}");
    assert!(stamp_path(root).exists());
    assert!(!installed_bin_path(root).exists());
    fs::remove_file(stamp_path(root)).unwrap();

    // bootstrap.sh's own stderr contract ("on failure exactly one line") checked directly.
    let direct1 = run_bootstrap(root, &[("RATCHET_RELEASE_BASE", &empty_base)]);
    assert_eq!(code(&direct1), 1);
    let derr1 = stderr(&direct1);
    assert_eq!(
        derr1.lines().count(),
        1,
        "expected exactly one stderr line from bootstrap.sh itself: {derr1}"
    );
    assert!(derr1.contains("download failed"), "{derr1}");
    fs::remove_file(stamp_path(root)).unwrap();

    // Run 2: SHA256SUMS.txt present, the asset missing — SHA256SUMS.txt is fetched first (I1a),
    // succeeds, and the failure still correctly names the asset it could not then download.
    let sums_only = TempDir::new().unwrap();
    let name = asset_name("0.1.0", host_target());
    fs::write(
        sums_only.path().join("SHA256SUMS.txt"),
        format!("{}  {name}\n", "0".repeat(64)),
    )
    .unwrap();
    let sums_only_base = file_url(sums_only.path());
    let out2 = run_wrapper(
        root,
        &["version"],
        &[("RATCHET_RELEASE_BASE", &sums_only_base)],
    );
    assert_eq!(code(&out2), 0);
    let err2 = stderr(&out2);
    assert!(
        err2.contains(&format!("download failed ({name})")),
        "{err2}"
    );
    assert!(stamp_path(root).exists());
    assert!(!installed_bin_path(root).exists());
    fs::remove_file(stamp_path(root)).unwrap();

    let direct2 = run_bootstrap(root, &[("RATCHET_RELEASE_BASE", &sums_only_base)]);
    assert_eq!(code(&direct2), 1);
    let derr2 = stderr(&direct2);
    assert_eq!(
        derr2.lines().count(),
        1,
        "expected exactly one stderr line from bootstrap.sh itself: {derr2}"
    );
    assert!(
        derr2.contains(&format!("download failed ({name})")),
        "{derr2}"
    );
    fs::remove_file(stamp_path(root)).unwrap();

    // Run 3: the asset present, SHA256SUMS.txt missing — SHA256SUMS.txt is fetched first (I1a)
    // and fails there, before the asset is ever requested.
    let asset_only = TempDir::new().unwrap();
    build_asset(asset_only.path(), "0.1.0", host_target());
    let asset_only_base = file_url(asset_only.path());
    let out3 = run_wrapper(
        root,
        &["version"],
        &[("RATCHET_RELEASE_BASE", &asset_only_base)],
    );
    assert_eq!(code(&out3), 0);
    let err3 = stderr(&out3);
    assert!(err3.contains("download failed (SHA256SUMS.txt)"), "{err3}");
    assert!(stamp_path(root).exists());
    assert!(!installed_bin_path(root).exists());
    fs::remove_file(stamp_path(root)).unwrap();

    let direct3 = run_bootstrap(root, &[("RATCHET_RELEASE_BASE", &asset_only_base)]);
    assert_eq!(code(&direct3), 1);
    let derr3 = stderr(&direct3);
    assert_eq!(
        derr3.lines().count(),
        1,
        "expected exactly one stderr line from bootstrap.sh itself: {derr3}"
    );
    assert!(
        derr3.contains("download failed (SHA256SUMS.txt)"),
        "{derr3}"
    );
}
