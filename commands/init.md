---
name: "ratchet: init"
description: "Opt this repo in to ratchet by writing a commented ratchet.toml at its root"
allowed-tools: Bash
category: "Setup"
tags: ["setup", "ratchet"]
---

Run `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" config init` with Bash from the repo root. That wrapper finds the ratchet binary the same way the hooks do (`RATCHET_BIN`, then the plugin's `bin/`).

- If it prints `wrote <path>`: tell the user the marker exists, show the file, and point out the three things they may want to change: `default_branch`, `worktrees_dir`, and `[guardrails] off`. Remind them the hooks start applying at the next tool call; no restart needed.
- If it says the file already exists: do not overwrite. Show the existing file and ask whether they want `--force`.
- If it says `binary not found`: the plugin's binary is not installed; point them to the README's Install section.
