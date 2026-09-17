//! `.claude-plugin/plugin.json` carries the same version as the crate. `Cargo.toml` is the
//! source of truth (spec §10 D-release-assets); the release workflow refuses a tag that does
//! not match both.

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

fn plugin_version() -> String {
    let text = fs::read_to_string(repo_root().join(".claude-plugin/plugin.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    doc["version"].as_str().unwrap().to_string()
}

#[test]
fn plugin_json_version_matches_cargo_toml() {
    assert_eq!(
        plugin_version(),
        cargo_version(),
        "plugin.json and Cargo.toml disagree on the version; Cargo.toml is the source of truth"
    );
}

#[test]
fn cargo_env_version_matches_cargo_toml() {
    // Belt and braces: the version compiled into the binary is the one in the manifest.
    assert_eq!(env!("CARGO_PKG_VERSION"), cargo_version());
}
