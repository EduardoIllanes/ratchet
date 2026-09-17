#!/usr/bin/env bash
# Downloads the ratchet release binary that matches this plugin's version, verifies it against
# the release's SHA256SUMS.txt, and unpacks it into <plugin-root>/bin/. Usage:
#   bash bootstrap.sh <plugin-root>
# Exit 0 on success (binary now in <root>/bin/); exit 1 on any failure. Never touches stdin.
# Prints nothing on stdout. On stderr: exactly one line on success, exactly one line on failure
# (except the "stamp is still young" retry-suppression path, which is silent).
set -u

root="${1:-}"
if [ -z "$root" ]; then
    root="$(cd "$(dirname "$0")/.." && pwd)"
fi

stamp="$root/bin/.bootstrap-failed"
asset_hint="the asset for your platform"
target=""

fail() {
    reason="$1"
    mkdir -p "$root/bin"
    touch "$stamp"
    printf '[ratchet] bootstrap failed: %s. Retry with: bash "%s/hooks/bootstrap.sh". Or download %s from https://github.com/EduardoIllanes/ratchet/releases into "%s/bin/", or set RATCHET_BIN.\n' \
        "$reason" "$root" "$asset_hint" "$root" 1>&2
    exit 1
}

# 0. Stamp: while younger than 60 minutes, stay completely silent and do not retry — checked
# before any other step so every failure kind (including a platform that will never resolve, or
# a plugin.json that can never be read) honours the 60-minute silence, not just download and
# checksum failures.
if [ -f "$stamp" ]; then
    if [ -n "$(find "$stamp" -mmin -60 2>/dev/null)" ]; then
        exit 1
    fi
    rm -f "$stamp"
fi

# 1. Version, from plugin.json.
version="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$root/.claude-plugin/plugin.json" 2>/dev/null | head -1)"
if [ -z "${version:-}" ]; then
    fail "cannot read version from plugin.json"
fi

# 2. Target: platform map, overridable by RATCHET_OS/RATCHET_ARCH for tests.
os="${RATCHET_OS:-$(uname -s)}"
arch="${RATCHET_ARCH:-$(uname -m)}"
ext="tar.gz"
bin="ratchet"
case "$os" in
    Darwin)
        case "$arch" in
            arm64) target="aarch64-apple-darwin" ;;
            x86_64) target="x86_64-apple-darwin" ;;
            *) fail "unsupported platform $os/$arch" ;;
        esac
        ;;
    Linux)
        case "$arch" in
            x86_64) target="x86_64-unknown-linux-gnu" ;;
            *) fail "unsupported platform $os/$arch" ;;
        esac
        ;;
    MINGW*|MSYS*|CYGWIN*)
        target="x86_64-pc-windows-msvc"
        ext="zip"
        bin="ratchet.exe"
        ;;
    *)
        fail "unsupported platform $os/$arch"
        ;;
esac

# 3. Base URL and asset name.
base="${RATCHET_RELEASE_BASE:-https://github.com/EduardoIllanes/ratchet/releases/download/v$version}"
asset="ratchet-$version-$target.$ext"
asset_hint="$asset"

# 4. Tools.
if ! command -v curl >/dev/null 2>&1; then
    fail "curl not found"
fi
if command -v shasum >/dev/null 2>&1; then
    sha_cmd="shasum -a 256"
elif command -v sha256sum >/dev/null 2>&1; then
    sha_cmd="sha256sum"
else
    fail "no sha256 tool"
fi

# 5. Mark an attempt in progress before downloading anything: two sequential downloads can
# take longer than a hook's own timeout, and a hard kill mid-download skips fail() entirely.
# Touching the stamp here first means 60 minutes of silence still follow even then.
mkdir -p "$root/bin"
touch "$stamp"

# Download into a temp dir, always removed on exit. SHA256SUMS.txt is small and fetched first,
# with a tight budget, so a slow or hanging host fails fast; the asset gets the larger share of
# the remaining time.
tmp="$(mktemp -d 2>/dev/null)"
if [ -z "${tmp:-}" ] || [ ! -d "$tmp" ]; then
    fail "download failed ($asset)"
fi
trap 'rm -rf "$tmp"' EXIT

if ! curl -fsSL --connect-timeout 2 --max-time 2 -o "$tmp/SHA256SUMS.txt" "$base/SHA256SUMS.txt" 2>/dev/null; then
    fail "download failed (SHA256SUMS.txt)"
fi
if ! curl -fsSL --connect-timeout 2 --max-time 6 -o "$tmp/$asset" "$base/$asset" 2>/dev/null; then
    fail "download failed ($asset)"
fi

# 6. Verify the checksum, case-insensitively. The release workflow writes SHA256SUMS.txt in
# text mode ("<hash>  <name>", two spaces, no leading "*"), which is what this awk lookup
# expects.
expected="$(awk -v a="$asset" '$NF==a {print $1}' "$tmp/SHA256SUMS.txt" 2>/dev/null | head -1)"
if [ -z "${expected:-}" ]; then
    fail "no checksum for $asset"
fi
actual="$($sha_cmd "$tmp/$asset" 2>/dev/null | awk '{print $1}')"
expected_lc="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
actual_lc="$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')"
if [ "$expected_lc" != "$actual_lc" ]; then
    fail "checksum mismatch for $asset"
fi

# 7. Unpack into a dedicated subdir; the archive must yield exactly one file, $bin.
extract_dir="$tmp/extract"
mkdir -p "$extract_dir"
case "$ext" in
    tar.gz)
        tar -C "$extract_dir" -xzf "$tmp/$asset" >/dev/null 2>&1 || fail "unexpected archive layout"
        ;;
    zip)
        if command -v unzip >/dev/null 2>&1; then
            unzip -q "$tmp/$asset" -d "$extract_dir" >/dev/null 2>&1 || fail "unexpected archive layout"
        else
            tar -C "$extract_dir" -xf "$tmp/$asset" >/dev/null 2>&1 || fail "unexpected archive layout"
        fi
        ;;
esac

entry_count="$(ls -A "$extract_dir" 2>/dev/null | grep -c .)"
if [ "$entry_count" -ne 1 ] || [ ! -f "$extract_dir/$bin" ]; then
    fail "unexpected archive layout"
fi

# 8. Install: stage into bin/ under a temp name, chmod it there, then rename atomically into
# place, checking every step.
mkdir -p "$root/bin"
tmp_name="$root/bin/.$bin.$$"
if ! mv -f "$extract_dir/$bin" "$tmp_name" 2>/dev/null; then
    fail "install failed (stage)"
fi
if ! chmod +x "$tmp_name" 2>/dev/null; then
    fail "install failed (chmod)"
fi
if ! mv -f "$tmp_name" "$root/bin/$bin" 2>/dev/null; then
    fail "install failed (rename)"
fi
rm -f "$stamp"
printf '[ratchet] installed ratchet %s (%s) into %s/bin/\n' "$version" "$target" "$root" 1>&2
exit 0
