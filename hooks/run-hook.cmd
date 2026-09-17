: << 'CMDBLOCK'
@echo off
REM Cross-platform wrapper. Claude Code invokes this with shell=bash on every platform;
REM cmd.exe only reaches the batch part if someone runs it by hand on Windows.
set "HOOK_DIR=%~dp0"
if exist "C:\Program Files\Git\bin\bash.exe" (
    "C:\Program Files\Git\bin\bash.exe" "%HOOK_DIR%run-hook.cmd" %*
    exit /b %ERRORLEVEL%
)
where bash >nul 2>nul
if %ERRORLEVEL% equ 0 (
    bash "%HOOK_DIR%run-hook.cmd" %*
    exit /b %ERRORLEVEL%
)
exit /b 0
CMDBLOCK

# bash from here on. Find the binary and exec it with the same args and stdin.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ -n "$RATCHET_BIN" ] && [ -x "$RATCHET_BIN" ]; then
    exec "$RATCHET_BIN" "$@"
fi
for candidate in "$ROOT/bin/ratchet" "$ROOT/bin/ratchet.exe"; do
    if [ -x "$candidate" ]; then
        exec "$candidate" "$@"
    fi
done
echo "[ratchet] binary not found. Build it (cargo build --release) and copy target/release/ratchet[.exe] into $ROOT/bin/, or set RATCHET_BIN." >&2
exit 0
