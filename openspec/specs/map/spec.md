# map

Repo orientation, derived deterministically from the repository itself — no model involved. See
`docs/superpowers/specs/2026-09-21-ratchet-map-design.md` for the full design.

## Purpose

Every session and every subagent starts by re-learning where things are. `ratchet map` derives a
capped, up-to-date map (`.ratchet/map.md`) from `git ls-files`, file headers and manifest files,
on demand, and reports its own freshness so nobody has to guess whether it's stale.

## Requirements

### Requirement: Map generation is deterministic
Two runs of `ratchet map` over the same tree, at the same commit, with the same recorded notes,
SHALL produce byte-identical `.ratchet/map.md` content.

#### Scenario: Two runs produce byte identical output
- **WHEN** `ratchet map` is run twice in a row over the same commit with `RATCHET_NOW` fixed
- **THEN** the two writes of `.ratchet/map.md` are byte-for-byte identical

### Requirement: `ratchet map` writes the map and reports on wiring
`ratchet map` SHALL write `.ratchet/map.md` and print one line naming the file, its line count
and how many modules have no description. Outside a repo with a `ratchet.toml` marker it SHALL
fail without writing anything. When `CLAUDE.md` does not import the map, or `.gitignore` does not
cover `.ratchet/`, it SHALL print one hint line per missing wire, naming `--wire`.

#### Scenario: Generating a map prints the wrote line
- **WHEN** `ratchet map` is run in a marker repo
- **THEN** it prints a line starting with `wrote ` naming `.ratchet/map.md`, its line count and
  the count of modules without a description, and the file exists afterward

#### Scenario: Map generation outside a marker repo fails
- **WHEN** `ratchet map` is run in a directory with no `ratchet.toml` above it
- **THEN** it fails, the message says the directory is not a ratchet-managed repo, and no
  `.ratchet/map.md` is written

#### Scenario: An unwired repo gets a hint naming wire
- **WHEN** `ratchet map` is run in a marker repo whose `CLAUDE.md` does not import the map and
  whose `.gitignore` does not cover `.ratchet/`
- **THEN** it prints two hint lines, each naming `--wire`

### Requirement: Header extraction per language
The first sentence of a file's leading header comment, cut at 120 characters, SHALL become its
module sentence: `//!` lines in Rust, a leading docstring in Python, a leading `/** */` or `//`
comment block in TypeScript.

#### Scenario: A Rust doc comment becomes the module sentence
- **WHEN** a tracked `.rs` file's first lines are a `//!` doc comment
- **THEN** the map's Modules section shows that file with the doc comment's first sentence

#### Scenario: A Python module docstring becomes the module sentence
- **WHEN** a tracked `.py` file opens with a triple-quoted module docstring
- **THEN** the map's Modules section shows that file with the docstring's first sentence

#### Scenario: A TypeScript leading comment becomes the module sentence
- **WHEN** a tracked `.ts` file opens with a `/** … */` block comment
- **THEN** the map's Modules section shows that file with the comment's first sentence

#### Scenario: A module with no header shows a placeholder
- **WHEN** a tracked source file has no recognised header comment and no note
- **THEN** the map's Modules section shows that file with a bare `—` in place of a sentence

### Requirement: Notes fill in where there is no header, and a header always wins
A note recorded through `ratchet map note` SHALL be shown for a file with no header comment. A
file that has both a header comment and a note SHALL show the header comment. A note for a path
that no longer exists in the tree SHALL be dropped, silently, the next time the map is generated.

#### Scenario: A note describes a header-less file
- **WHEN** a note is recorded for a tracked file with no header comment, and the map is
  regenerated
- **THEN** the map's Modules section shows that file with the note's sentence

#### Scenario: A header always wins over a note
- **WHEN** a note is recorded for a tracked file that also has a header comment, and the map is
  regenerated
- **THEN** the map's Modules section shows that file with the header comment's sentence, not the
  note

#### Scenario: A stale note is dropped at the next generation
- **WHEN** a note exists for a path that has since been deleted from the tree, and the map is
  regenerated
- **THEN** the generation succeeds and the note for the deleted path no longer appears in
  `.ratchet/map.notes`

### Requirement: `map note` records one description, or refuses
`ratchet map note <path> "<sentence>"` SHALL record one sentence for a tracked file, replacing
any existing note for the same path. It SHALL refuse, with exit code 1 and one stderr line and
without changing `.ratchet/map.notes`, a path that is not a tracked file, a sentence over 120
characters or containing a newline, or an empty sentence.

#### Scenario: map note records a sentence for a tracked file
- **WHEN** `ratchet map note` is run naming a tracked file and a short sentence
- **THEN** it exits 0 and `.ratchet/map.notes` records that sentence for that path

#### Scenario: map note replaces an existing note for the same path
- **WHEN** `ratchet map note` is run twice for the same tracked file with two different sentences
- **THEN** `.ratchet/map.notes` holds only the second sentence for that path

#### Scenario: map note refuses a path that is not tracked
- **WHEN** `ratchet map note` is run naming a path that is not a tracked file
- **THEN** it exits 1, the message says the path is not a tracked file, and
  `.ratchet/map.notes` is unchanged

#### Scenario: map note refuses an invalid sentence
- **WHEN** `ratchet map note` is run with a sentence over 120 characters, and separately with a
  sentence containing a newline
- **THEN** both calls exit 1, the message says the sentence must be a single line of at most 120
  characters, and `.ratchet/map.notes` is unchanged

#### Scenario: map note refuses an empty sentence
- **WHEN** `ratchet map note` is run with an empty sentence
- **THEN** it exits 1, the message says the sentence must not be empty, and
  `.ratchet/map.notes` is unchanged

### Requirement: `--missing` lists undescribed source files, full and incremental
`ratchet map --missing` SHALL list, one path per line, the tracked source files with neither a
header comment nor a note. With a map already present, it SHALL narrow that list to files changed
since the map's recorded commit; `--all` SHALL widen it back to every such file regardless of the
recorded commit.

#### Scenario: Missing lists every file with no header and no note
- **WHEN** `ratchet map --missing` is run with no map present yet, over a tree with some files
  that have headers and some that don't
- **THEN** it lists exactly the files with neither a header nor a note, one per line

#### Scenario: Missing narrows to files changed since the recorded commit
- **WHEN** a map exists, a new commit adds one more header-less file, and `ratchet map --missing`
  is run
- **THEN** it lists only the file added since the map's recorded commit, not the header-less files
  that already existed when the map was generated

#### Scenario: Missing --all widens back to every undescribed file
- **WHEN** the same repo as the previous scenario is queried with `ratchet map --missing --all`
- **THEN** it lists every header-less, note-less file in the tree, not just the one added since
  the recorded commit

### Requirement: A map over the line cap collapses directories, deepest first
When the generated map would exceed 150 lines, the Modules section SHALL collapse the deepest
directories' file lists into one summary line each (`<dir>/ — <n> files`), deepest first, until
the map fits the cap or nothing more can be collapsed.

#### Scenario: A module list over the cap collapses into directory counts
- **WHEN** a tree has enough tracked source files that an uncollapsed map would exceed 150 lines
- **THEN** the generated map is at most 150 lines, and its Modules section shows at least one
  `<dir>/ — <n> files` summary line instead of per-file lines for that directory

### Requirement: `--wire` writes the import and the gitignore entry, idempotently
`ratchet map --wire` SHALL append `.ratchet/` to `.gitignore` (creating it if absent) and append
an import block to `CLAUDE.md` (creating it if absent) that imports `.ratchet/map.md`, and SHALL
change neither file when run again with both already in place. It SHALL refuse, without writing
the map, when `CLAUDE.md` or `.gitignore` exists but is not a regular file.

#### Scenario: Wire appends the CLAUDE.md import and the gitignore entry
- **WHEN** `ratchet map --wire` is run in a marker repo with neither file wired yet
- **THEN** `CLAUDE.md` now imports `.ratchet/map.md` and `.gitignore` now covers `.ratchet/`

#### Scenario: Wire run twice changes nothing the second time
- **WHEN** `ratchet map --wire` is run a second time immediately after the first
- **THEN** neither `CLAUDE.md` nor `.gitignore` changes, and the command says both are already
  wired

#### Scenario: Wire refuses a symlinked CLAUDE.md
- **WHEN** `CLAUDE.md` is a symlink instead of a regular file
- **THEN** `ratchet map --wire` fails, the message says it is not a regular file, and neither
  `CLAUDE.md` nor `.gitignore` is modified

### Requirement: The briefing reports the map's freshness
Session start SHALL append one line to the briefing when a marker repo has no map, a map whose
recorded commit is behind `HEAD`, or a map recorded from a commit that is not an ancestor of
`HEAD`. A current map SHALL add no line.

#### Scenario: No map prints the map none line
- **WHEN** a session starts in a marker repo with no `.ratchet/map.md`
- **THEN** the briefing contains the line `map: none — run /ratchet:map for the repo layout`

#### Scenario: A map behind HEAD prints the commits behind line
- **WHEN** a map is generated, one more commit is made, and a session starts
- **THEN** the briefing contains a line of the form `map: 1 commits behind — run /ratchet:map`

#### Scenario: A map from another branch prints the from another branch line
- **WHEN** a map is generated, then the branch is reset so the map's recorded commit is no longer
  an ancestor of `HEAD`, and a session starts
- **THEN** the briefing contains the line `map: from another branch — run /ratchet:map`

#### Scenario: A current map prints no line
- **WHEN** a map is generated and a session starts immediately after, with no further commits
- **THEN** the briefing contains no line starting with `map:`

### Requirement: `ratchet map status` mirrors the briefing line
`ratchet map status` SHALL print, in a marker repo, exactly the line the briefing would show, or
`map: current` when the map is up to date. Outside a marker repo it SHALL exit 0 and print
nothing.

#### Scenario: Map status prints the same line the briefing would show
- **WHEN** a map is behind `HEAD` and `ratchet map status` is run
- **THEN** it prints exactly the same `map: … commits behind …` line the briefing would show

#### Scenario: Map status prints nothing outside a marker repo
- **WHEN** `ratchet map status` is run in a directory with no `ratchet.toml` above it
- **THEN** it exits 0 and prints nothing

### Requirement: `[map]` config overrides exclusion and gate detection
`[map] exclude` in `ratchet.toml` SHALL keep matching files out of the map entirely. `[map] gate`
SHALL replace detected gate commands with the configured list, verbatim.

#### Scenario: Map exclude leaves matching files out of the map
- **WHEN** `[map] exclude` names a glob matching a tracked file, and the map is generated
- **THEN** that file appears nowhere in the generated map

#### Scenario: Map gate replaces detected gate commands entirely
- **WHEN** `[map] gate` is set to a custom command list in a repo that would otherwise detect a
  Cargo-based gate
- **THEN** the map's Gate section shows exactly the configured commands and none of the detected
  ones
