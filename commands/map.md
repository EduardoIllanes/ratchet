---
name: "ratchet: map"
description: "Regenerate the repo map that CLAUDE.md imports, and optionally describe files with no header"
allowed-tools: Bash, Agent
category: "Setup"
tags: ["setup", "ratchet", "map"]
---

Run `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" map` with Bash from the repo root.

- Report the `wrote ...` line verbatim.
- If it printed one or two `hint:` lines naming `--wire`, show them and ask the user whether to
  run `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" map --wire`. Only run it on a clear yes — it
  edits `CLAUDE.md` and `.gitignore` at the repo root.
- If the argument is `--deep`: run `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" map --missing`.
  If it prints nothing, say the map already describes every file and stop. Otherwise dispatch
  the `mapper` agent with the printed list, in batches of at most 40 paths, one dispatch at a
  time — never in parallel, since every batch writes through the same
  `.ratchet/map.notes`. After each batch completes, run
  `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" map` again and report the new footer counts
  (`<n> with a header, <m> with a note, <k> without either`).
- If it says `binary not found`: point the user to the README's Install section.
