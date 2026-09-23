: << 'CMDBLOCK'
@echo off
REM Cross-platform wrapper. On macOS/Linux Claude Code runs the command line through /bin/sh,
REM which needs the exec bit on this file (kept in git as 100755); cmd.exe reaches the batch part on Windows.
set "HOOK_DIR=%~dp0"
REM No parenthesised blocks below. cmd.exe expands %ERRORLEVEL% once, when it parses an entire
REM parenthesised block, before anything inside that block has run - so a block that starts bash
REM and then does exit /b %ERRORLEVEL% always reports the errorlevel from before bash ran. That
REM was the bug. No goto or labels either, since those are unreliable in a batch file that must
REM stay LF-terminated, which this file is - pinned in .gitattributes, because the sh heredoc
REM terminator below depends on it. No delayed expansion, since a literal exclamation point in
REM hook arguments would be stripped before bash ever saw it. So: one statement per line, with
REM %ERRORLEVEL% read only on the line right after the one that set it.
set "GIT_BASH="
REM Prefer the Git Bash that ships next to whatever git.exe is on PATH. git.exe can live in the
REM install root's cmd, bin or mingw64\bin folder; bash.exe always lives at root\bin\bash.exe.
for /f "delims=" %%G in ('where git 2^>nul') do if not defined GIT_BASH if exist "%%~dpG..\bin\bash.exe" set "GIT_BASH=%%~dpG..\bin\bash.exe"
for /f "delims=" %%G in ('where git 2^>nul') do if not defined GIT_BASH if exist "%%~dpG..\..\bin\bash.exe" set "GIT_BASH=%%~dpG..\..\bin\bash.exe"
if not defined GIT_BASH if exist "%ProgramFiles%\Git\bin\bash.exe" set "GIT_BASH=%ProgramFiles%\Git\bin\bash.exe"
if not defined GIT_BASH if exist "%LOCALAPPDATA%\Programs\Git\bin\bash.exe" set "GIT_BASH=%LOCALAPPDATA%\Programs\Git\bin\bash.exe"
REM Never fall back to a bare bash resolved from PATH. CreateProcess searches
REM C:\Windows\System32 before PATH, where bash.exe is the WSL launcher, not Git Bash -
REM see resolve_bash's comment in tests/spec/bootstrap.rs.
if defined GIT_BASH "%GIT_BASH%" "%HOOK_DIR%run-hook.cmd" %*
if defined GIT_BASH exit /b %ERRORLEVEL%
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
