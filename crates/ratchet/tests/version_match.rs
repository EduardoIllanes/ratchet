//! `.claude-plugin/binary-version` pins the same version as the crate. `Cargo.toml` is the
//! source of truth (spec §10 D-release-assets); the release workflow refuses a tag that does
//! not match both. `plugin.json` deliberately carries no version: Claude Code then tracks the
//! marketplace commit, so agents and skills update without a binary release.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn cargo_version() -> String {
    let text = fs::read_to_string(repo_root().join("crates/ratchet/Cargo.toml")).unwrap();
    let doc: toml::Value = toml::from_str(&text).unwrap();
    doc["package"]["version"].as_str().unwrap().to_string()
}

fn pinned_binary_version() -> String {
    let text = fs::read_to_string(repo_root().join(".claude-plugin/binary-version")).unwrap();
    text.trim().to_string()
}

#[test]
fn binary_version_file_matches_cargo_toml() {
    assert_eq!(
        pinned_binary_version(),
        cargo_version(),
        "binary-version and Cargo.toml disagree; Cargo.toml is the source of truth"
    );
}

#[test]
fn plugin_json_carries_no_version() {
    let text = fs::read_to_string(repo_root().join(".claude-plugin/plugin.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        doc.get("version").is_none(),
        "plugin.json must not pin a version: it would stop Claude Code from picking up agent and skill changes until the next release"
    );
}

#[test]
fn cargo_env_version_matches_cargo_toml() {
    // Belt and braces: the version compiled into the binary is the one in the manifest.
    assert_eq!(env!("CARGO_PKG_VERSION"), cargo_version());
}
