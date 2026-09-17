: << 'CMDBLOCK'
@echo off
REM Cross-platform wrapper. On macOS/Linux Claude Code runs the command line through /bin/sh,
REM which needs the exec bit on this file (kept in git as 100755); cmd.exe reaches the batch part on Windows.
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
try_exec() {
    for candidate in "$ROOT/bin/ratchet" "$ROOT/bin/ratchet.exe"; do
        if [ -x "$candidate" ]; then
            exec "$candidate" "$@"
        fi
    done
}
try_exec "$@"
bash "$ROOT/hooks/bootstrap.sh" "$ROOT" </dev/null
try_exec "$@"
echo "[ratchet] binary not found. Bootstrap did not install it (see the line above, if any). Build from source (cargo build --release) and copy the binary into $ROOT/bin/, or set RATCHET_BIN." >&2
exit 0
