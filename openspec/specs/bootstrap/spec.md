# bootstrap

Installing the plugin must be enough: the hooks call one small binary, `ratchet`, which is not
committed to this repository and must be in place before any hook can do useful work.

## Purpose

The plugin ships hooks and markdown only. `bin/ratchet` arrives on first use: this capability
downloads the release asset for the current platform matching the version pinned in
`.claude-plugin/binary-version`, verifies it against the release's checksums, and unpacks it
into `bin/`, so adding the plugin is the only step a user ever has to take. A failure here must never break a Claude Code session.

## Requirements

### Requirement: Binary bootstrap on first run
When no binary is found (`RATCHET_BIN` unset or not executable, nothing in `bin/`), the hook
wrapper SHALL run `hooks/bootstrap.sh`, which SHALL download the release asset for the current
platform matching the version pinned in `.claude-plugin/binary-version`, verify it against the
release's `SHA256SUMS.txt`, unpack it into `bin/`, and the wrapper SHALL then run it. Any failure SHALL leave the session
untouched: exit 0, one stderr line naming the manual path, and a stamp that keeps later hooks
silent for 60 minutes. An existing binary SHALL never be replaced by bootstrap. `plugin.json`
SHALL carry no `version`, so Claude Code tracks the marketplace commit and changes to agents,
skills and hooks reach users without a binary release.

#### Scenario: First run downloads, verifies and runs the binary
- **WHEN** the wrapper runs `version` with an empty `bin/` and `RATCHET_RELEASE_BASE` pointing at a directory holding a valid archive and sums file
- **THEN** stdout is `ratchet <version>`, `bin/` now holds the binary, stderr has one `installed` line, and the stamp is absent

#### Scenario: Checksum mismatch refuses the download
- **WHEN** the sums file lists a wrong hash for the asset
- **THEN** the wrapper exits 0, stderr has one line containing `checksum mismatch`, `bin/` holds no binary, and the stamp exists

#### Scenario: A failed bootstrap is silent until the stamp expires
- **WHEN** the previous attempt failed and the wrapper runs again within 60 minutes
- **THEN** it exits 0 and prints exactly one line, the wrapper's own not-found line, with no bootstrap line; and after the stamp is aged past 60 minutes a valid release is installed on the next run

#### Scenario: Unsupported platform is reported once
- **WHEN** `RATCHET_OS=Plan9` and `bin/` is empty
- **THEN** the wrapper exits 0, stderr contains `unsupported platform`, and the stamp exists; and
  a second run within 60 minutes prints only the wrapper's not-found line, with no bootstrap line

#### Scenario: An existing binary is never re-downloaded
- **WHEN** `bin/` already holds the binary and `RATCHET_RELEASE_BASE` points at a directory that does not exist
- **THEN** the wrapper runs it, stdout is `ratchet <version>`, and no stamp is created

#### Scenario: Download failure names the manual path
- **WHEN** `RATCHET_RELEASE_BASE` points at a directory without the asset
- **THEN** the wrapper exits 0, stderr has one line containing `download failed` and `RATCHET_BIN`, and the stamp exists
