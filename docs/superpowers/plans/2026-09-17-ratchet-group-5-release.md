# ratchet — Group 5: CI, release assets and manual install

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `v0.1.0` of the plugin as a GitHub release with one binary per platform and a
checksum file, gated by a CI workflow that runs the full test gate on Ubuntu, macOS and
Windows, and a README that tells a user without a Rust toolchain how to install the plugin
and point the hooks at the binary (`RATCHET_BIN` or the plugin's `bin/`).

**Architecture:** Two GitHub Actions workflows under `.github/workflows/`: `ci.yml` (gate on
every push and pull request to `main`) and `release.yml` (on tags `v*`: verify the tag matches
both version files, build four targets, publish archives plus `SHA256SUMS.txt`). No bootstrap:
`hooks/run-hook.cmd` stays exactly as it is (spec D-no-bootstrap). One new integration test
pins `plugin.json`'s version to `Cargo.toml`'s. The content layer (skills, agents, README) is
reconciled against the CLI the binary really exposes.

**Tech Stack:** GitHub Actions with `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`,
`Swatinem/rust-cache@v2`, `actions/upload-artifact@v4`, `actions/download-artifact@v4`,
`softprops/action-gh-release@v2`. Rust 2021 (rust-version 1.79), existing deps only
(`serde_json`, `toml` are already regular dependencies and are visible to integration tests).
`gh` CLI, authenticated, for pushing, opening the PR and watching runs.

**Spec:** `docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md` — §2 D-binary-delivery
as amended by §10 (D-no-bootstrap, D-release-assets, D-ci-gate, D-check-scenarios-in-cargo,
"Group 5 scope"), §3 layout, §6 error handling, §7 testing, §8 group 5.

## Global Constraints

- **Git is allowed and expected in this repo now.** The owner works on macOS from this account
  (previous groups ran on a Windows machine with a no-git rule; that rule is void here). Work
  on the branch `g5-release`, commit per task, push, and open one PR. Never force-push. Never
  commit to `main` directly; `main` receives the PR merge and the release tag only.
- Every shell that runs cargo starts with:
  ```bash
  export PATH="$HOME/.cargo/bin:$PATH"
  cd /Users/eduardoillanes/Documents/ratchet
  ```
- The crate is **bin-only**: unit tests run with `cargo test -p ratchet --bin ratchet`. Never
  `--lib`; this plan adds no `[lib]` target.
- Gate, unchanged from groups 0-4 and green on this Mac at the start of group 5 (141 bin, 1
  cli_version, 2 db_open, 1 latency, 3 scenarios, 87 spec):
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`.
- All content, identifiers, messages and docs in English (spec D-english).
- Version source of truth: `crates/ratchet/Cargo.toml` `[package] version`. `.claude-plugin/plugin.json`
  `version` must equal it. Both are `0.1.0` today and stay `0.1.0` for this group.
- Release asset names, fixed: `ratchet-<version>-<target>.tar.gz` for the three Unix targets,
  `ratchet-<version>-<target>.zip` for Windows, and `SHA256SUMS.txt`. `<version>` has no `v`
  prefix (`0.1.0`); the tag has it (`v0.1.0`). Targets, exactly:
  `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu`. Each archive contains only the binary at its top level
  (`ratchet` or `ratchet.exe`).
- The latency test (`crates/ratchet/tests/latency.rs`, function `pre_tool_median_under_ceiling`)
  is **not** part of the CI gate. CI runs it in release with `--nocapture` and
  `continue-on-error: true`. Do not change its ceiling or its code.
- Do not touch `hooks/run-hook.cmd`, `hooks/hooks.json`, or anything under `crates/ratchet/src/`.
  Group 5 changes no runtime behaviour.
- Do not create `hooks/bootstrap.sh` or `scripts/` (spec §10).

---

## File structure

```
ratchet/
├── .github/workflows/
│   ├── ci.yml                       Task 2: gate on 3 OSes + latency report
│   └── release.yml                  Task 4: tag → version check → 4 builds → release
├── .claude-plugin/
│   ├── plugin.json                  unchanged (version pinned by Task 1's test)
│   └── marketplace.json             Task 3: single-plugin marketplace so `claude plugin
│                                    marketplace add EduardoIllanes/ratchet` works
├── crates/ratchet/tests/
│   └── version_match.rs             Task 1: plugin.json version == Cargo.toml version
├── skills/ratchet-tasks/SKILL.md    Task 5: reconciled against `ratchet task --help`
├── skills/ratchet-pdf/SKILL.md      Task 5: reconciled against `ratchet pdf --help`
├── agents/*.md, docs/agent-doctrine.md   Task 5: same
└── README.md                        Task 3: Install rewritten; Task 5: CLI mentions
```

Task order: 1 (test) → 2 (ci.yml, so the PR gets a green check) → 3 (README + marketplace) →
4 (release.yml) → 5 (content pass) → 6 (PR, merge, tag, verify release) → 7 (clean install
on this Mac).

---

### Task 1: version-match test

**Files:**
- Create: `crates/ratchet/tests/version_match.rs`

**Interfaces:**
- Consumes: `crates/ratchet/Cargo.toml` (`[package] version`), `.claude-plugin/plugin.json`
  (`"version"`), both relative to `CARGO_MANIFEST_DIR` the same way
  `crates/ratchet/tests/scenarios.rs::repo_root()` does.
- Produces: nothing other tasks call. Task 4's workflow re-checks the same equality in shell;
  this test is the local, always-on form.

- [ ] **Step 1: Create the branch**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
git switch -c g5-release
```

- [ ] **Step 2: Write the test**

`crates/ratchet/tests/version_match.rs`:

```rust
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
```

- [ ] **Step 3: Run it, expect PASS (both files say 0.1.0)**

```bash
cargo test -p ratchet --test version_match
```
Expected: `test result: ok. 2 passed`.

- [ ] **Step 4: Prove the test is not vacuous**

Temporarily break the plugin manifest, run, then restore:

```bash
sed -i '' 's/"version": "0.1.0"/"version": "0.0.0"/' .claude-plugin/plugin.json
cargo test -p ratchet --test version_match 2>&1 | grep -E "panicked|test result"
git checkout -- .claude-plugin/plugin.json
git diff --stat   # must print nothing for plugin.json
```
Expected in the middle line: `plugin_json_version_matches_cargo_toml ... FAILED` and
`test result: FAILED. 1 passed; 1 failed`. After the checkout, `git diff --stat` shows no
change to `plugin.json`.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test -p ratchet
git add crates/ratchet/tests/version_match.rs
git commit -m "test: pin plugin.json version to Cargo.toml

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the gate commands from Global Constraints; the latency test name
  `pre_tool_median_under_ceiling`.
- Produces: a required status check named `gate` (matrix job) that Task 6 waits on.

- [ ] **Step 1: Write the workflow**

`.github/workflows/ci.yml`:

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: -D warnings

jobs:
  gate:
    name: gate (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: fmt
        run: cargo fmt --check
      - name: clippy
        run: cargo clippy --all-targets -- -D warnings
      - name: test (gate; latency excluded)
        run: cargo test -p ratchet -- --skip pre_tool_median_under_ceiling
      - name: latency (report only)
        continue-on-error: true
        run: cargo test -p ratchet --release --test latency -- --nocapture
```

Notes for the implementer, not for the file: `--skip` is a test-harness filter and applies to
every test binary cargo runs, so the one latency test is skipped and everything else
(bin unit tests, cli_version, db_open, scenarios, spec) runs. `RUSTFLAGS: -D warnings` makes
a stray warning fail the build on every OS; the local gate already demands the same via
clippy.

- [ ] **Step 2: Validate the YAML locally**

```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('yaml ok')"
```
Expected: `yaml ok`. (If `yaml` is missing: `python3 -m pip install --user pyyaml`, or skip:
GitHub reports a parse error within seconds of the push in Step 4.)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: gate on ubuntu, macos and windows; latency as report

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

- [ ] **Step 4: Push and watch the first run**

```bash
git push -u origin g5-release
gh pr create --base main --head g5-release --title "Group 5: CI, release workflow, manual install" --body "$(cat <<'EOF'
Group 5 per spec §8 and §10: CI gate on three OSes, release workflow on tags, version-match test, README install for users without a toolchain, content consistency pass.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
gh pr checks --watch
```
Expected: three `gate (...)` checks, all passing. If Windows fails on something the local
Mac gate cannot show (path separators, `\r\n`), fix it in the test or workflow, commit as
`ci: fix <what> on windows`, push, watch again. Do not weaken a test to get green: if a test
is wrong on Windows, the fix is in how the test computes its expectation, never in dropping
the assertion.

---

### Task 3: README install rewrite and marketplace manifest

**Files:**
- Create: `.claude-plugin/marketplace.json`
- Modify: `README.md` — sections `## Build from source (until releases exist)` (lines 10-13),
  `## Install (group 0, from source)` (lines 37-55), `## Not here (yet)` (lines 181-186).

**Interfaces:**
- Consumes: the asset names and targets from Global Constraints (the README must quote them
  exactly, since Task 4 produces them).
- Produces: the install procedure Task 7 follows verbatim on a clean machine.

- [ ] **Step 1: Write the marketplace manifest**

`.claude-plugin/marketplace.json`:

```json
{
  "name": "ratchet",
  "owner": { "name": "Eduardo Illanes" },
  "plugins": [
    {
      "name": "ratchet",
      "source": "./",
      "description": "Guardrails, task board and spec-driven agent roles for Claude Code, enforced by hooks instead of prompts."
    }
  ]
}
```

- [ ] **Step 2: Check it parses and matches plugin.json**

```bash
python3 - <<'EOF'
import json
m = json.load(open('.claude-plugin/marketplace.json'))
p = json.load(open('.claude-plugin/plugin.json'))
assert m['plugins'][0]['name'] == p['name'] == 'ratchet'
assert m['plugins'][0]['description'] == p['description']
print('marketplace ok')
EOF
```
Expected: `marketplace ok`.

- [ ] **Step 3: Replace the two install sections with one**

Delete `## Build from source (until releases exist)` and its two indented lines, and delete
`## Install (group 0, from source)` through the paragraph ending `--nocapture`.` (the latency
paragraph). In the place of the first deleted section, insert:

````markdown
## Install

The plugin is markdown plus hooks; the hooks call one binary, `ratchet`, which is **not**
committed to this repo and is **not** downloaded automatically. You put it in place once.

1. Add the marketplace and install the plugin:

       claude plugin marketplace add EduardoIllanes/ratchet
       claude plugin install ratchet@ratchet

   Or, for a checkout of this repo, run Claude Code with `--plugin-dir /path/to/ratchet`.

2. Get the binary for your platform from the release that matches the plugin version
   (`.claude-plugin/plugin.json`; today `0.1.0`), from
   https://github.com/EduardoIllanes/ratchet/releases:

   | Platform | Asset |
   |---|---|
   | macOS Apple Silicon | `ratchet-0.1.0-aarch64-apple-darwin.tar.gz` |
   | macOS Intel | `ratchet-0.1.0-x86_64-apple-darwin.tar.gz` |
   | Linux x64 | `ratchet-0.1.0-x86_64-unknown-linux-gnu.tar.gz` |
   | Windows x64 | `ratchet-0.1.0-x86_64-pc-windows-msvc.zip` |

   Verify it against `SHA256SUMS.txt` from the same release and unpack the single file it
   contains. Then tell the hooks where it is. Two ways:

   **a. `RATCHET_BIN` (recommended).** Keep the binary at a path of your own and export the
   variable in the shell Claude Code starts from (`~/.zshrc`, `~/.bashrc`, or the Windows
   user environment). The hooks check it first. This survives plugin updates. On macOS:

       V=0.1.0; T=aarch64-apple-darwin
       mkdir -p ~/.ratchet/bin && cd "$(mktemp -d)"
       curl -sSLO https://github.com/EduardoIllanes/ratchet/releases/download/v$V/ratchet-$V-$T.tar.gz
       curl -sSLO https://github.com/EduardoIllanes/ratchet/releases/download/v$V/SHA256SUMS.txt
       shasum -a 256 --check --ignore-missing SHA256SUMS.txt
       tar xzf ratchet-$V-$T.tar.gz && mv ratchet ~/.ratchet/bin/
       echo 'export RATCHET_BIN="$HOME/.ratchet/bin/ratchet"' >> ~/.zshrc

   (On Linux use `sha256sum --check --ignore-missing`; on Windows, `certutil -hashfile
   ratchet-0.1.0-x86_64-pc-windows-msvc.zip SHA256` and compare by eye, unzip
   `ratchet.exe` somewhere stable, and set `RATCHET_BIN` to its full path.)

   **b. `bin/` of the installed plugin.** Claude Code installs each plugin version in its own
   directory, so this has to be redone after every plugin update. The directory is the
   `installPath` for `ratchet` in `~/.claude/plugins/installed_plugins.json`:

       P="$(python3 -c 'import json,os;d=json.load(open(os.path.expanduser("~/.claude/plugins/installed_plugins.json")));print(d["plugins"]["ratchet@ratchet"][0]["installPath"])')"
       mv ratchet "$P/bin/"

   Until the binary is in place every hook prints one line, `[ratchet] binary not found …`,
   and exits 0. Nothing is blocked and nothing is recorded.

3. Restart the Claude Code session. In a repo you want governed, create `ratchet.toml` at its
   root (see "Opt a repo in"). Check from that repo:

       ratchet version                      # ratchet 0.1.0
       ratchet guardrails list
       ratchet guardrails test Bash '{"command":"python x.py"}'   # exit 2 if the repo has a .venv

### Build from source

With a Rust toolchain (1.79 or newer; on macOS also the Xcode Command Line Tools, on Windows
the Visual Studio Build Tools, both for the bundled SQLite):

    cargo build --release
    cp target/release/ratchet <plugin dir>/bin/     # ratchet.exe on Windows

### Latency

Measured `pre-tool` cost on Windows 11 (release build): about 11-13 ms of ratchet's own work
above process-launch cost (full `pre-tool` runs ~53-56 ms in a direct harness against a ~40 ms
do-nothing-binary floor on that machine). The check this replaces, in a Python harness, cost
~830 ms. `cargo test -p ratchet --release --test latency -- --nocapture` prints the figures for
your machine; the 60 ms ceiling in that test is a local sanity check and fails on slow
launchers, which is why CI reports it without gating on it.
````

Keep the `## Opt a repo in` section that follows exactly where it is. The install section must
sit before it, because step 3 refers to it.

- [ ] **Step 4: Update "Not here (yet)"**

Replace the whole `## Not here (yet)` section with:

```markdown
## Not here (yet)

No automatic download of the binary: the hooks never fetch anything, by decision (spec §10
D-no-bootstrap). Binaries are not code-signed or notarized; macOS Gatekeeper may ask once
(`xattr -d com.apple.quarantine bin/ratchet` clears it). No Linux arm64 build. The audited
web-fetch flow (approval lists, robots.txt, cache) that the original group 3 plan would have
ported is deliberately not planned for `ratchet` — it stays in `ops`.
```

Also change line 7's `Status: group 2 — …` line to:

```markdown
Status: v0.1.0 — groups 0-5 of the design spec are shipped (guardrails, state, task board,
`ratchet pdf`, content, release). See
`docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md`.
```

- [ ] **Step 5: Check the README renders and has no stale headings**

```bash
grep -n "^#" README.md
grep -n -E "group 0|until releases exist|bootstrap" README.md
```
Expected: headings `Install`, `Build from source`, `Latency`, `Opt a repo in`, `Agents and
skills`, `Guardrails`, `PDF`, `State and sessions`, `The board`, `Not here (yet)` in that
order (Install before Opt a repo in), and the second grep prints only the `D-no-bootstrap`
line from "Not here (yet)".

- [ ] **Step 6: Commit and push**

```bash
git add .claude-plugin/marketplace.json README.md
git commit -m "docs: install from releases, marketplace manifest

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
git push
```

---

### Task 4: release workflow

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: asset names and targets from Global Constraints; `Cargo.toml` and `plugin.json`
  versions.
- Produces: a GitHub release for tag `vX.Y.Z` with five assets. Task 6 exercises it.

- [ ] **Step 1: Write the workflow**

`.github/workflows/release.yml`:

```yaml
name: release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

env:
  CARGO_TERM_COLOR: always

jobs:
  check-version:
    name: tag matches Cargo.toml and plugin.json
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.v.outputs.version }}
    steps:
      - uses: actions/checkout@v4
      - id: v
        shell: bash
        run: |
          tag="${GITHUB_REF_NAME#v}"
          cargo_v="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/ratchet/Cargo.toml | head -1)"
          plugin_v="$(python3 -c 'import json;print(json.load(open(".claude-plugin/plugin.json"))["version"])')"
          echo "tag=$tag cargo=$cargo_v plugin=$plugin_v"
          if [ "$tag" != "$cargo_v" ] || [ "$tag" != "$plugin_v" ]; then
            echo "::error::tag v$tag does not match Cargo.toml ($cargo_v) and plugin.json ($plugin_v)"
            exit 1
          fi
          echo "version=$tag" >> "$GITHUB_OUTPUT"

  build:
    name: build (${{ matrix.target }})
    needs: check-version
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: true
      matrix:
        include:
          - { os: ubuntu-latest,  target: x86_64-unknown-linux-gnu, ext: tar.gz, bin: ratchet }
          - { os: macos-latest,   target: aarch64-apple-darwin,     ext: tar.gz, bin: ratchet }
          - { os: macos-latest,   target: x86_64-apple-darwin,      ext: tar.gz, bin: ratchet }
          - { os: windows-latest, target: x86_64-pc-windows-msvc,   ext: zip,    bin: ratchet.exe }
    env:
      VERSION: ${{ needs.check-version.outputs.version }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.target }}
      - name: build
        run: cargo build --release -p ratchet --target ${{ matrix.target }}
      - name: smoke
        shell: bash
        run: |
          # The x86_64 macOS binary is cross-built on an arm64 runner; Rosetta runs it there.
          out="$(target/${{ matrix.target }}/release/${{ matrix.bin }} version)"
          echo "$out"
          [ "$out" = "ratchet $VERSION" ]
      - name: package (unix)
        if: matrix.ext == 'tar.gz'
        shell: bash
        run: |
          name="ratchet-$VERSION-${{ matrix.target }}"
          tar -C "target/${{ matrix.target }}/release" -czf "$name.tar.gz" ratchet
          ls -l "$name.tar.gz"
      - name: package (windows)
        if: matrix.ext == 'zip'
        shell: pwsh
        run: |
          $name = "ratchet-$env:VERSION-${{ matrix.target }}"
          Compress-Archive -Path "target/${{ matrix.target }}/release/ratchet.exe" -DestinationPath "$name.zip"
          Get-Item "$name.zip"
      - uses: actions/upload-artifact@v4
        with:
          name: ratchet-${{ env.VERSION }}-${{ matrix.target }}
          path: ratchet-${{ env.VERSION }}-${{ matrix.target }}.${{ matrix.ext }}
          if-no-files-found: error

  publish:
    name: publish release
    needs: [check-version, build]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with:
          path: dist
          merge-multiple: true
      - name: checksums
        shell: bash
        run: |
          cd dist
          ls -l
          sha256sum ratchet-*.tar.gz ratchet-*.zip > SHA256SUMS.txt
          cat SHA256SUMS.txt
      - uses: softprops/action-gh-release@v2
        with:
          name: ratchet ${{ needs.check-version.outputs.version }}
          generate_release_notes: true
          files: |
            dist/ratchet-*.tar.gz
            dist/ratchet-*.zip
            dist/SHA256SUMS.txt
```

Notes for the implementer: `merge-multiple: true` flattens the four artifacts into `dist/`.
`sha256sum` output is `<hash>  <name>`, which `shasum -a 256 --check` and
`sha256sum --check` both read (the README relies on this). The smoke step runs the freshly
built binary so a broken cross-build fails before packaging.

- [ ] **Step 2: Validate the YAML and the version-check shell locally**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('yaml ok')"
GITHUB_REF_NAME=v0.1.0 bash -c '
  tag="${GITHUB_REF_NAME#v}"
  cargo_v="$(sed -n "s/^version = \"\(.*\)\"/\1/p" crates/ratchet/Cargo.toml | head -1)"
  plugin_v="$(python3 -c "import json;print(json.load(open(\".claude-plugin/plugin.json\"))[\"version\"])")"
  echo "tag=$tag cargo=$cargo_v plugin=$plugin_v"
  [ "$tag" = "$cargo_v" ] && [ "$tag" = "$plugin_v" ] && echo match'
GITHUB_REF_NAME=v9.9.9 bash -c '
  tag="${GITHUB_REF_NAME#v}"
  cargo_v="$(sed -n "s/^version = \"\(.*\)\"/\1/p" crates/ratchet/Cargo.toml | head -1)"
  [ "$tag" = "$cargo_v" ] || echo mismatch-detected'
```
Expected: `yaml ok`, then `tag=0.1.0 cargo=0.1.0 plugin=0.1.0` and `match`, then
`mismatch-detected`. Note `head -1`: `crates/ratchet/Cargo.toml` has exactly one
`version = "…"` line at column 0 today (dependency versions are inline tables or quoted
after `= {`), and `head -1` keeps the package one first regardless.

- [ ] **Step 3: Dry-run the macOS packaging on this Mac**

```bash
cargo build --release -p ratchet --target aarch64-apple-darwin
[ "$(target/aarch64-apple-darwin/release/ratchet version)" = "ratchet 0.1.0" ] && echo smoke-ok
tar -C target/aarch64-apple-darwin/release -czf /tmp/ratchet-0.1.0-aarch64-apple-darwin.tar.gz ratchet
tar tzf /tmp/ratchet-0.1.0-aarch64-apple-darwin.tar.gz
shasum -a 256 /tmp/ratchet-0.1.0-aarch64-apple-darwin.tar.gz
rm /tmp/ratchet-0.1.0-aarch64-apple-darwin.tar.gz
```
Expected: `smoke-ok`, then `tar tzf` prints exactly one entry, `ratchet`.

- [ ] **Step 4: Commit and push**

```bash
git add .github/workflows/release.yml
git commit -m "ci: release workflow on v* tags with four targets and checksums

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
git push
gh pr checks --watch
```
Expected: the `gate` checks still green (the release workflow does not run on PRs).

---

### Task 5: content consistency pass (deferred from group 4)

**Files:**
- Modify (only where a discrepancy is found): `skills/ratchet-tasks/SKILL.md`,
  `skills/ratchet-pdf/SKILL.md`, `agents/analyst.md`, `agents/spec-test-author.md`,
  `agents/implementer.md`, `agents/reviewer.md`, `agents/researcher.md`,
  `docs/agent-doctrine.md`, `README.md`.

**Interfaces:**
- Consumes: the real CLI, via `--help` of the debug binary.
- Produces: content that names only commands, subcommands and flags the binary accepts.

- [ ] **Step 1: Dump the real CLI surface**

```bash
cargo build -p ratchet
B=target/debug/ratchet
S=/private/tmp/claude-501/-Users-eduardoillanes-Documents-ratchet/d347ffde-adcb-41dc-9d31-37542b077b01/scratchpad
mkdir -p "$S"
{
  $B --help
  for c in task session db guardrails pdf; do echo "=== $c ==="; $B $c --help; done
  for s in list show new claim status check note handoff archive unarchive; do echo "=== task $s ==="; $B task $s --help; done
  for s in list show; do echo "=== session $s ==="; $B session $s --help; done
} > "$S/cli-help.txt" 2>&1
wc -l "$S/cli-help.txt"
```

- [ ] **Step 2: List every CLI mention in the content layer**

```bash
grep -rnoE "ratchet (task|session|pdf|db|guardrails|hook|config|version)( [a-zA-Z0-9_.-]+)*( --?[a-z-]+( [^ \`]+)?)*" \
  skills agents docs/agent-doctrine.md README.md > "$S/cli-mentions.txt"
wc -l "$S/cli-mentions.txt"
```

- [ ] **Step 3: Compare, one mention at a time**

For each line of `cli-mentions.txt`: the subcommand must appear as a subcommand in
`cli-help.txt` and every flag must appear in that subcommand's help. Known at planning time,
to verify first:

- `ratchet task board` (one mention). There is no `board` subcommand; the list is
  `list|show|new|claim|status|check|note|handoff|archive|unarchive`. If the sentence means
  "look at the board", rewrite it around `ratchet task list`.
- `ratchet session` bare (one mention). Valid only if the sentence is about the group of
  commands; if it is shown as something to run, make it `ratchet session list` or `show`.
- `ratchet pdf report.pdf --pages "1-2"`: the greedy grep catches the file name; this one is
  valid, confirm `--pages` is in `pdf --help`.
- `ratchet task list --mine`, `--all`: confirm both flags exist in `task list --help`.
  (`--all` may be the "include archived" flag or may not exist; the help decides.)
- `ratchet session list --live`: confirm `--live` exists.

Fix each real discrepancy in place with the smallest edit that makes the sentence true. Do
not restructure paragraphs. Do not add commands the content does not need.

- [ ] **Step 4: Check status vocabulary and ids**

```bash
grep -rnoE "\b(backlog|ready|in_progress|blocked|review|done)\b" skills/ratchet-tasks/SKILL.md | sort | uniq -c
grep -rnoE "T-[0-9]{4}" skills agents docs/agent-doctrine.md README.md | head
$B task status --help
```
Expected: the six statuses in the skill match the six the CLI lists; every task id example
has the form `T-0001`.

- [ ] **Step 5: Gate and commit**

```bash
cargo test -p ratchet --test scenarios   # content edits cannot break it, prove it anyway
git add -A skills agents docs/agent-doctrine.md README.md
git commit -m "docs: reconcile skills and agents with the shipped CLI

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
git push
```
If Step 3 found nothing to change, commit nothing and record "no discrepancies" plus the
mention count in the task report.

---

### Task 6: merge, tag, verify the release

**Files:** none created. Git and GitHub state only.

**Interfaces:**
- Consumes: green `gate` checks on the PR; `release.yml` from Task 4.
- Produces: release `v0.1.0` with five assets, which Task 7 installs from.

- [ ] **Step 1: Confirm the PR is green and merge it**

```bash
gh pr checks
gh pr merge --squash --delete-branch
git switch main && git pull
git log --oneline -3
```
Expected: `gh pr checks` shows three passing `gate` checks before the merge; after `pull`,
`main` has the squash commit on top of `0421210 Initial commit`.

- [ ] **Step 2: Tag and push the tag**

```bash
git tag -a v0.1.0 -m "ratchet 0.1.0"
git push origin v0.1.0
gh run list --workflow release --limit 1
gh run watch "$(gh run list --workflow release --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status
```
Expected: `check-version`, four `build (...)` and `publish release` all succeed. If a build
fails: fix in a new branch, PR, merge, then delete the tag locally and remotely
(`git tag -d v0.1.0 && git push origin :refs/tags/v0.1.0`), delete the failed release if one
was created (`gh release delete v0.1.0 --yes`), and re-tag. Never move a tag that already
has a published release with assets that someone could have downloaded; for `0.1.0` before
Task 7 nobody has, so re-tagging is acceptable this once.

- [ ] **Step 3: Verify the assets and checksums from this Mac**

```bash
gh release view v0.1.0 --json assets -q '.assets[].name'
D="$(mktemp -d)"; cd "$D"
gh release download v0.1.0 -p 'ratchet-*' -p 'SHA256SUMS.txt'
shasum -a 256 --check SHA256SUMS.txt
tar tzf ratchet-0.1.0-aarch64-apple-darwin.tar.gz
tar xzf ratchet-0.1.0-aarch64-apple-darwin.tar.gz && ./ratchet version
tar xzf ratchet-0.1.0-x86_64-apple-darwin.tar.gz && ./ratchet version   # runs under Rosetta
unzip -l ratchet-0.1.0-x86_64-pc-windows-msvc.zip
cd - && rm -rf "$D"
```
Expected: five names (four archives plus `SHA256SUMS.txt`); four `OK` lines from `shasum`;
each tar lists exactly `ratchet`; both macOS binaries print `ratchet 0.1.0`; the zip lists
exactly `ratchet.exe`.

---

### Task 7: clean install on this Mac, following the README only

**Files:** none in the repo. If a README step turns out wrong, fix it in a branch and PR
as `docs: fix install step <n>`.

- [ ] **Step 1: Hide the toolchain and any local binary**

```bash
export PATH="$(echo "$PATH" | tr ':' '\n' | grep -v '.cargo/bin' | paste -sd: -)"
command -v cargo || echo "no cargo on PATH: good"
unset RATCHET_BIN
```

- [ ] **Step 2: Install the plugin from GitHub as the README says**

```bash
claude plugin marketplace add EduardoIllanes/ratchet
claude plugin install ratchet@ratchet
claude plugin list
```
Expected: `ratchet@ratchet` listed as installed and enabled. (`claude plugin list` does not
print the path; the README's `P=…` line reads it from `installed_plugins.json`.)

- [ ] **Step 3: First session without the binary**

Create a throwaway governed repo and start a session:

```bash
R="$(mktemp -d)/demo"; mkdir -p "$R" && cd "$R" && git init -q
printf '[repo]\ndefault_branch = "main"\n' > ratchet.toml
claude -p "say ok" 2>&1 | head -5
```
Expected: one line `[ratchet] binary not found. Build it (cargo build --release) and copy …`
on stderr, the model answers, nothing else. (This is the spec §6 "missing binary" path.)

- [ ] **Step 4: Place the binary as the README says**

Run README step 2, way **b** (the plugin's `bin/`), verbatim, with `V=0.1.0;
T=aarch64-apple-darwin` and the `P=…` line as printed. Then:

```bash
"$P/bin/ratchet" version
cd "$R" && claude -p "say ok" 2>&1 | head -20
```

Then also prove way **a** without editing `~/.zshrc`: move the binary out of `bin/` to
`~/.ratchet/bin/ratchet` and run the session with the variable set inline:

```bash
mkdir -p ~/.ratchet/bin && mv "$P/bin/ratchet" ~/.ratchet/bin/
cd "$R" && RATCHET_BIN="$HOME/.ratchet/bin/ratchet" claude -p "say ok" 2>&1 | head -20
```
Expected: the same `[ratchet]` briefing both times.
Expected: `ratchet 0.1.0`; the session prints the `[ratchet]` briefing (repo · session ·
branch, "nothing pending" or similar) instead of the not-found line.

- [ ] **Step 5: Guardrail proof and cleanup**

```bash
cd "$R" && mkdir .venv
~/.ratchet/bin/ratchet guardrails test Bash '{"command":"python x.py"}'; echo "exit $?"
cd / && rm -rf "$(dirname "$R")"
claude plugin uninstall ratchet@ratchet   # optional; keep it if the owner wants it installed
```
Expected: `exit 2`.

- [ ] **Step 6: Record**

Write the observed plugin path, the briefing text and the exit code into the task report.
If any README step needed a correction, the correction is a PR (`docs: fix install step
<n>`), merged, and no re-tag is needed (the README is not in the release assets).

---

## Self-review against the spec

- §8 group 5 "Cross-platform build workflow" → Task 4. "Release assets with checksums" →
  Task 4 publish job, verified in Task 6. "Bootstrap download path exercised end to end on a
  clean machine" → replaced per §10 D-no-bootstrap by the manual path, exercised in Task 7.
- §10 D-release-assets version check → Task 4 `check-version` job and Task 1's test.
- §10 D-ci-gate → Task 2 (three OSes, latency as report).
- §10 D-check-scenarios-in-cargo → nothing to build; `tests/scenarios.rs` runs inside Task 2's
  `cargo test`.
- §10 scope "README install rewrite plus the marketplace manifest" → Task 3.
- §10 scope "deferred group-4 consistency pass" → Task 5.
- §6 "Missing binary → exit 0 with a single stderr line" → observed in Task 7 Step 3, code
  untouched.
- Names used across tasks: asset names and targets identical in Tasks 3, 4, 6, 7; test
  function `pre_tool_median_under_ceiling` in Task 2 matches `tests/latency.rs:9`; branch
  `g5-release` in Tasks 1, 2, 6.
