//! Deterministic repo orientation: `git ls-files`, file headers, manifests and a small local
//! notes file, written to `<repo root>/.ratchet/map.md`. No model is involved anywhere in this
//! file — the `mapper` agent (Task 5) only ever calls `ratchet map note`, the same command a
//! person can run by hand.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{DateTime, Utc};

use crate::config::MapSection;
use crate::repo::is_tracked;

/// A map is context in every prompt; it never grows with the repo (design D-map-cap).
pub const MAP_LINE_CAP: usize = 150;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapError(String);

impl MapError {
    // `cli::map_cmd::fail` takes `impl Display` (so it also accepts `ConfigError` from
    // `repo::find_repo`) and prints via `{e}`, not `.message()` — this accessor is kept for
    // parity with `pdf::PdfError` (whose callers do use `.message()`) and for any future caller
    // that wants the bare string without the `Display` wrapper.
    #[allow(dead_code)]
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MapError {}

fn err(msg: impl Into<String>) -> MapError {
    MapError(msg.into())
}

pub fn map_path(main_root: &Path) -> PathBuf {
    main_root.join(".ratchet").join("map.md")
}

pub fn notes_path(main_root: &Path) -> PathBuf {
    main_root.join(".ratchet").join("map.notes")
}

// --- git plumbing ---------------------------------------------------------------------------

fn git_output(main_root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", &main_root.to_string_lossy()])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn tracked_files(main_root: &Path) -> Result<Vec<String>, MapError> {
    let out = Command::new("git")
        .args(["-C", &main_root.to_string_lossy(), "ls-files"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .output()
        .map_err(|e| err(format!("git ls-files failed: {e}")))?;
    if !out.status.success() {
        return Err(err("git ls-files failed"));
    }
    let mut files: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.replace('\\', "/"))
        .filter(|l| !l.is_empty())
        .collect();
    files.sort();
    Ok(files)
}

// --- `[map] exclude` -------------------------------------------------------------------------

/// Translates a glob (`*` matches any run of characters, including `/`) into an anchored regex
/// and checks `rel_path` against it. No new dependency: reuses `fancy-regex`, already a crate
/// dependency for the guardrail rules.
pub fn is_excluded(rel_path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| {
        let mut re = String::from("^");
        for ch in p.chars() {
            match ch {
                '*' => re.push_str(".*"),
                '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                    re.push('\\');
                    re.push(ch);
                }
                other => re.push(other),
            }
        }
        re.push('$');
        fancy_regex::Regex::new(&re)
            .ok()
            .and_then(|r| r.is_match(rel_path).ok())
            .unwrap_or(false)
    })
}

// --- header extraction ------------------------------------------------------------------------

enum Style {
    RustDoc,
    PyDocstring,
    BlockOrLineComment,
    ShellShebang,
}

/// Design §3.4's language table. TS/JS/Go/Java/Kotlin/Swift/C/C++ share one comment style.
const LANG_TABLE: &[(&[&str], Style)] = &[
    (&["rs"], Style::RustDoc),
    (&["py"], Style::PyDocstring),
    (
        &[
            "ts", "tsx", "js", "jsx", "go", "java", "kt", "swift", "c", "h", "cpp", "hpp", "cc",
        ],
        Style::BlockOrLineComment,
    ),
    (&["sh", "bash"], Style::ShellShebang),
];

fn style_for(ext: &str) -> Option<&'static Style> {
    LANG_TABLE
        .iter()
        .find(|(exts, _)| exts.contains(&ext))
        .map(|(_, s)| s)
}

/// First sentence, cut at 120 characters (design §3.4).
fn cut_sentence(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len());
    let cut = s[..end].trim();
    let cut = cut.trim_end_matches('.');
    if cut.chars().count() > 119 {
        format!("{}…", cut.chars().take(119).collect::<String>())
    } else {
        format!("{cut}.")
    }
}

fn read_head(path: &Path, n: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = fs::File::open(path).ok()?;
    let mut buf = vec![0u8; n];
    let got = f.read(&mut buf).ok()?;
    buf.truncate(got);
    Some(buf)
}

/// The file's header sentence, or `None` when it has no header this table recognises, or the
/// head is not valid UTF-8. Reads at most 4 KB (design §4.1).
fn header_sentence(path: &Path) -> Option<String> {
    let bytes = read_head(path, 4096)?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let style = style_for(&ext)?;
    let raw = match style {
        Style::RustDoc => {
            let lines: Vec<&str> = text.lines().take_while(|l| l.starts_with("//!")).collect();
            if lines.is_empty() {
                return None;
            }
            lines
                .iter()
                .map(|l| l.trim_start_matches("//!").trim())
                .collect::<Vec<_>>()
                .join(" ")
        }
        Style::PyDocstring => {
            let t = text.trim_start();
            let quote = if t.starts_with("\"\"\"") {
                "\"\"\""
            } else if t.starts_with("'''") {
                "'''"
            } else {
                return None;
            };
            let rest = &t[quote.len()..];
            let end = rest.find(quote)?;
            rest[..end].split_whitespace().collect::<Vec<_>>().join(" ")
        }
        Style::BlockOrLineComment => {
            let t = text.trim_start();
            if let Some(rest) = t.strip_prefix("/**").or_else(|| t.strip_prefix("/*")) {
                let end = rest.find("*/")?;
                rest[..end]
                    .lines()
                    .map(|l| l.trim().trim_start_matches('*').trim())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else if t.starts_with("//") {
                let lines: Vec<&str> = t
                    .lines()
                    .take_while(|l| l.trim_start().starts_with("//"))
                    .collect();
                lines
                    .iter()
                    .map(|l| l.trim_start().trim_start_matches("//").trim())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                return None;
            }
        }
        Style::ShellShebang => {
            let mut lines = text.lines();
            if !lines.next()?.starts_with("#!") {
                return None;
            }
            let rest: Vec<&str> = lines.take_while(|l| l.starts_with('#')).collect();
            if rest.is_empty() {
                return None;
            }
            rest.iter()
                .map(|l| l.trim_start_matches('#').trim())
                .collect::<Vec<_>>()
                .join(" ")
        }
    };
    if raw.trim().is_empty() {
        None
    } else {
        Some(cut_sentence(&raw))
    }
}

// --- notes -----------------------------------------------------------------------------------

/// `path: sentence` per line (design D-map-notes). Malformed lines are skipped, not an error —
/// a hand-edited or partially-written notes file must never break generation.
fn load_notes(main_root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(text) = fs::read_to_string(notes_path(main_root)) else {
        return out;
    };
    for line in text.lines() {
        if let Some((path, sentence)) = line.split_once(": ") {
            out.insert(path.to_string(), sentence.to_string());
        }
    }
    out
}

fn write_notes(main_root: &Path, notes: &BTreeMap<String, String>) -> Result<(), MapError> {
    let path = notes_path(main_root);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| err(format!("could not create .ratchet/: {e}")))?;
    }
    let mut text = String::new();
    for (p, s) in notes {
        text.push_str(&format!("{p}: {s}\n"));
    }
    fs::write(&path, text).map_err(|e| err(format!("could not write {}: {e}", path.display())))
}

/// Drops, silently, any note whose path is no longer in `all_files` (the full tracked list,
/// before `[map] exclude`), rewriting `.ratchet/map.notes` only when something was actually
/// dropped. Called once per `generate()` — this is the one place a stale note gets pruned.
fn prune_notes(
    main_root: &Path,
    all_files: &[String],
) -> Result<BTreeMap<String, String>, MapError> {
    let notes = load_notes(main_root);
    if notes.is_empty() {
        return Ok(notes);
    }
    let tracked: std::collections::BTreeSet<&str> = all_files.iter().map(String::as_str).collect();
    let pruned: BTreeMap<String, String> = notes
        .iter()
        .filter(|(k, _)| tracked.contains(k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if pruned.len() != notes.len() {
        write_notes(main_root, &pruned)?;
    }
    Ok(pruned)
}

/// Records one description for a tracked file, replacing any existing note for the same path.
/// `target` may be absolute or relative to the process cwd — the CLI face resolves it before
/// calling this; what lands in `.ratchet/map.notes` is always the repo-root-relative,
/// case-preserving form (`repo::rel_for_git`), so the notes file stays portable.
pub fn note(main_root: &Path, target: &Path, sentence: &str) -> Result<(), MapError> {
    if sentence.is_empty() {
        return Err(err("sentence must not be empty"));
    }
    if sentence.contains('\n') || sentence.chars().count() > 120 {
        return Err(err(
            "sentence must be a single line of at most 120 characters",
        ));
    }
    if !is_tracked(main_root, target) {
        return Err(err(format!("not a tracked file: {}", target.display())));
    }
    let rel = crate::repo::rel_for_git(main_root, target)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .ok_or_else(|| err(format!("not a tracked file: {}", target.display())))?;
    let mut notes = load_notes(main_root);
    notes.insert(rel, sentence.to_string());
    write_notes(main_root, &notes)
}

/// Source files with neither a header nor a note. Full list when `all` or no map exists yet;
/// otherwise narrowed to files that changed since the map's recorded commit
/// (`git diff --name-only <recorded>..HEAD`, design §4.1).
pub fn missing(main_root: &Path, cfg: &MapSection, all: bool) -> Result<Vec<String>, MapError> {
    let all_files = tracked_files(main_root)?;
    let files: Vec<String> = all_files
        .into_iter()
        .filter(|f| !is_excluded(f, &cfg.exclude))
        .collect();
    let (_, source_dirs) = layout_section(&files);
    let notes = load_notes(main_root);
    let mut candidates = Vec::new();
    for f in &files {
        let ext = Path::new(f)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let recognised_lang = ext
            .as_deref()
            .map(|e| style_for(e).is_some())
            .unwrap_or(false);
        let under_source = source_dirs.iter().any(|d| f.starts_with(&format!("{d}/")));
        if !recognised_lang && !under_source {
            continue;
        }
        if notes.contains_key(f) {
            continue;
        }
        if header_sentence(&main_root.join(f)).is_some() {
            continue;
        }
        candidates.push(f.clone());
    }
    if all {
        return Ok(candidates);
    }
    let Some(recorded) = recorded_commit(main_root) else {
        return Ok(candidates); // no map yet: the full list either way
    };
    let changed = git_output(
        main_root,
        &["diff", "--name-only", &format!("{recorded}..HEAD")],
    )
    .unwrap_or_default();
    let changed_set: std::collections::BTreeSet<&str> = changed.lines().collect();
    Ok(candidates
        .into_iter()
        .filter(|f| changed_set.contains(f.as_str()))
        .collect())
}

// --- gate detection (unit-tested directly; only the Cargo branch is scenario-tested) ---------

fn detect_gate(root: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    if root.join("Cargo.toml").is_file() {
        lines.push("cargo fmt --all -- --check".to_string());
        lines.push("cargo clippy --all-targets -- -D warnings".to_string());
        lines.push("cargo test".to_string());
    }
    if root.join("pyproject.toml").is_file() || root.join("pytest.ini").is_file() {
        let uses_uv = root.join("uv.lock").is_file();
        lines.push(if uses_uv {
            "uv run pytest".to_string()
        } else {
            "pytest".to_string()
        });
        let pyproject = fs::read_to_string(root.join("pyproject.toml")).unwrap_or_default();
        if pyproject.contains("[tool.ruff]") {
            lines.push("ruff check".to_string());
        }
        if pyproject.contains("[tool.mypy]") {
            lines.push("mypy".to_string());
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let runner = if root.join("pnpm-lock.yaml").is_file() {
                "pnpm"
            } else if root.join("yarn.lock").is_file() {
                "yarn"
            } else {
                "npm"
            };
            if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
                for script in ["test", "lint", "typecheck"] {
                    if scripts.contains_key(script) {
                        lines.push(format!("{runner} run {script}"));
                    }
                }
            }
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("Makefile")) {
        for target in ["test", "lint"] {
            if text.lines().any(|l| l.starts_with(&format!("{target}:"))) {
                lines.push(format!("make {target}"));
            }
        }
    }
    if root.join("go.mod").is_file() {
        lines.push("go test ./...".to_string());
        lines.push("go vet ./...".to_string());
    }
    lines
}

fn gate_section(root: &Path, cfg: &MapSection) -> Vec<String> {
    if !cfg.gate.is_empty() {
        cfg.gate.clone()
    } else {
        detect_gate(root)
    }
}

// --- layout ------------------------------------------------------------------------------------

fn summarize(files: &[&String]) -> (usize, String) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        if let Some(ext) = Path::new(f.as_str()).extension().and_then(|e| e.to_str()) {
            *counts.entry(format!(".{ext}")).or_insert(0) += 1;
        }
    }
    let mut by_count: Vec<(String, usize)> = counts.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top2: Vec<String> = by_count.into_iter().take(2).map(|(e, _)| e).collect();
    (files.len(), top2.join(" "))
}

const SOURCE_MARKERS: &[&str] = &["src", "crates", "lib", "app", "cmd", "pkg", "packages"];
const ENTRY_POINTS: &[&str] = &[
    "main.rs",
    "lib.rs",
    "__main__.py",
    "main.py",
    "index.ts",
    "index.js",
    "main.go",
];

/// Returns the Layout section's text and the list of directories the Modules section should
/// treat as "source" (top-level dirs the manifest marks as source, plus one level under
/// `crates/*`/`packages/*`) — design §3.3.
fn layout_section(files: &[String]) -> (String, Vec<String>) {
    let mut top: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for f in files {
        if let Some((dir, _)) = f.split_once('/') {
            top.entry(dir.to_string()).or_default().push(f);
        }
    }
    let mut lines = Vec::new();
    let mut source_dirs = Vec::new();
    for (dir, dir_files) in &top {
        let (n, exts) = summarize(dir_files);
        lines.push(format!("{dir}/  {n} files  {exts}"));
        if SOURCE_MARKERS.contains(&dir.as_str()) {
            source_dirs.push(dir.clone());
        }
    }
    for dir in ["crates", "packages"] {
        if !top.contains_key(dir) {
            continue;
        }
        let mut subs: BTreeMap<String, Vec<&String>> = BTreeMap::new();
        for f in &top[dir] {
            let mut parts = f.splitn(3, '/');
            parts.next();
            if let (Some(sub), Some(_rest)) = (parts.next(), parts.next()) {
                subs.entry(format!("{dir}/{sub}")).or_default().push(f);
            }
        }
        for (sub, sub_files) in &subs {
            let (n, exts) = summarize(sub_files);
            lines.push(format!("  {sub}/  {n} files  {exts}"));
            source_dirs.push(sub.clone());
        }
    }
    let entries: Vec<&str> = files
        .iter()
        .filter(|f| {
            let name = f.rsplit('/').next().unwrap_or(f);
            ENTRY_POINTS.contains(&name) || f.starts_with("bin/")
        })
        .map(String::as_str)
        .collect();
    if !entries.is_empty() {
        lines.push(format!("Entry points: {}", entries.join(", ")));
    }
    if lines.is_empty() {
        (String::new(), source_dirs)
    } else {
        (format!("## Layout\n\n{}\n", lines.join("\n")), source_dirs)
    }
}

// --- modules -------------------------------------------------------------------------------

fn modules_section(
    root: &Path,
    files: &[String],
    source_dirs: &[String],
    notes: &BTreeMap<String, String>,
) -> (Vec<(PathBuf, String)>, usize, usize, usize) {
    let mut rows = Vec::new();
    let (mut with_header, mut with_note, mut without) = (0, 0, 0);
    for f in files {
        let ext = Path::new(f)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let recognised_lang = ext
            .as_deref()
            .map(|e| style_for(e).is_some())
            .unwrap_or(false);
        let under_source = source_dirs.iter().any(|d| f.starts_with(&format!("{d}/")));
        if !recognised_lang && !under_source {
            continue;
        }
        let sentence = if let Some(h) = header_sentence(&root.join(f)) {
            with_header += 1;
            h
        } else if let Some(n) = notes.get(f) {
            with_note += 1;
            n.clone()
        } else {
            without += 1;
            "—".to_string()
        };
        rows.push((PathBuf::from(f), format!("{f} — {sentence}")));
    }
    (rows, with_header, with_note, without)
}

/// Collapses the deepest directories first (most path components; ties broken by directory path,
/// ascending, for determinism), replacing their per-file lines with one `<dir>/ — <n> files`
/// line, until `overshoot` lines have been saved or nothing more can be collapsed.
fn collapse_modules(rows: &[(PathBuf, String)], overshoot: usize) -> Vec<String> {
    let mut by_dir: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for (path, _) in rows {
        *by_dir
            .entry(path.parent().unwrap_or(Path::new("")).to_path_buf())
            .or_insert(0) += 1;
    }
    let mut dirs: Vec<PathBuf> = by_dir.keys().cloned().collect();
    dirs.sort_by(|a, b| {
        b.components()
            .count()
            .cmp(&a.components().count())
            .then(a.cmp(b))
    });
    let mut collapsed: std::collections::BTreeSet<PathBuf> = Default::default();
    let mut saved = 0usize;
    for dir in &dirs {
        if saved >= overshoot {
            break;
        }
        let n = by_dir[dir];
        if n <= 1 {
            continue; // collapsing one file to one line saves nothing
        }
        collapsed.insert(dir.clone());
        saved += n - 1;
    }
    let mut out = Vec::new();
    let mut emitted: std::collections::BTreeSet<PathBuf> = Default::default();
    for (path, line) in rows {
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        if collapsed.contains(&dir) {
            if emitted.insert(dir.clone()) {
                out.push(format!("{}/ — {} files", dir.display(), by_dir[&dir]));
            }
        } else {
            out.push(line.clone());
        }
    }
    out
}

// --- tests, docs -----------------------------------------------------------------------------

fn tests_section(files: &[String]) -> String {
    let mut by_test_dir: BTreeMap<String, usize> = BTreeMap::new();
    let mut beside: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        if f.starts_with("tests/") || f.contains("/tests/") {
            let dir = f
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_else(|| "tests".into());
            *by_test_dir.entry(dir).or_insert(0) += 1;
            continue;
        }
        let name = f.rsplit('/').next().unwrap_or(f);
        let beside_style = (name.starts_with("test_") && name.ends_with(".py"))
            || name.ends_with("_test.go")
            || name.ends_with(".test.ts");
        if beside_style {
            let dir = f
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_else(|| ".".into());
            *beside.entry(dir).or_insert(0) += 1;
        }
    }
    if by_test_dir.is_empty() && beside.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    for (dir, n) in &by_test_dir {
        lines.push(format!("{dir}/  {n} files"));
    }
    for (dir, n) in &beside {
        lines.push(format!("{dir}/  {n} test files"));
    }
    format!("## Tests\n\n{}\n", lines.join("\n"))
}

fn docs_section(files: &[String]) -> String {
    let mut lines = Vec::new();
    for name in ["README.md", "CLAUDE.md", "AGENTS.md"] {
        if files.iter().any(|f| f == name) {
            lines.push(name.to_string());
        }
    }
    let docs_count = files.iter().filter(|f| f.starts_with("docs/")).count();
    if docs_count > 0 {
        lines.push(format!("docs/  {docs_count} files"));
    }
    let specs: std::collections::BTreeSet<String> = files
        .iter()
        .filter_map(|f| f.strip_prefix("openspec/specs/"))
        .filter_map(|rest| rest.split('/').next())
        .map(str::to_string)
        .collect();
    if !specs.is_empty() {
        lines.push(format!(
            "openspec/specs: {}",
            specs.into_iter().collect::<Vec<_>>().join(", ")
        ));
    } else if files.iter().any(|f| f.starts_with("openspec/")) {
        lines.push("openspec/  present".to_string());
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("## Docs\n\n{}\n", lines.join("\n"))
    }
}

// --- wiring hints (detection only — the `--wire` write path is Task 5) -----------------------

fn wiring_hints(main_root: &Path) -> Vec<String> {
    let mut hints = Vec::new();
    let imports = fs::read_to_string(main_root.join("CLAUDE.md"))
        .map(|t| t.contains("@.ratchet/map.md"))
        .unwrap_or(false);
    if !imports {
        hints
            .push("hint: CLAUDE.md does not import the map — run `ratchet map --wire`".to_string());
    }
    let covers = fs::read_to_string(main_root.join(".gitignore"))
        .map(|t| {
            t.lines()
                .any(|l| matches!(l.trim(), ".ratchet/" | ".ratchet"))
        })
        .unwrap_or(false);
    if !covers {
        hints.push(
            "hint: .gitignore does not cover .ratchet/ — run `ratchet map --wire`".to_string(),
        );
    }
    hints
}

// --- generation ----------------------------------------------------------------------------

pub struct Generated {
    pub text: String,
    // Kept for API completeness alongside `without` (which `cli::map_cmd::generate` does read);
    // no caller in this plan reads these two back off the struct — the same counts are already
    // baked into `text`'s footer line.
    #[allow(dead_code)]
    pub with_header: usize,
    #[allow(dead_code)]
    pub with_note: usize,
    pub without: usize,
    pub hints: Vec<String>,
}

pub fn generate(
    main_root: &Path,
    cfg: &MapSection,
    now: DateTime<Utc>,
) -> Result<Generated, MapError> {
    let all_files = tracked_files(main_root)?;
    let files: Vec<String> = all_files
        .iter()
        .filter(|f| !is_excluded(f, &cfg.exclude))
        .cloned()
        .collect();
    // A note for a path no longer in the tree is dropped here, silently, before it can leak
    // into this run's Modules section or linger in `.ratchet/map.notes` (spec: "Notes fill in
    // where there is no header, and a header always wins" — the stale-note scenario). Pruned
    // against the full tracked list, not the post-`[map] exclude` `files`, so a note for a file
    // that merely became excluded (still tracked, just hidden from the map) survives.
    let notes = prune_notes(main_root, &all_files)?;

    let repo_name = main_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let short_sha = git_output(main_root, &["rev-parse", "--short", "HEAD"])
        .ok_or_else(|| err("git rev-parse failed"))?;
    let full_sha =
        git_output(main_root, &["rev-parse", "HEAD"]).ok_or_else(|| err("git rev-parse failed"))?;
    let tree_sha = git_output(main_root, &["rev-parse", "HEAD^{tree}"]).unwrap_or_default();

    let header = format!(
        "# {repo_name} — map\n\ngenerated by ratchet map at {short_sha} ({}); do not edit, run /ratchet:map\n",
        now.format("%Y-%m-%d")
    );
    let gate_lines = gate_section(main_root, cfg);
    let gate = if gate_lines.is_empty() {
        String::new()
    } else {
        format!("\n## Gate\n\n{}\n", gate_lines.join("\n"))
    };

    let (layout, source_dirs) = layout_section(&files);
    let (rows, with_header, with_note, without) =
        modules_section(main_root, &files, &source_dirs, &notes);
    let tests = tests_section(&files);
    let docs = docs_section(&files);

    let render_modules = |lines: &[String]| -> String {
        if lines.is_empty() {
            String::new()
        } else {
            format!("\n## Modules\n\n{}\n", lines.join("\n"))
        }
    };
    let module_lines: Vec<String> = rows.iter().map(|(_, l)| l.clone()).collect();
    let mut body = format!(
        "{header}{gate}\n{layout}{}{tests}\n{docs}\n",
        render_modules(&module_lines)
    );

    const FOOTER_LINES: usize = 2; // sections 1, 2, 7 never collapse (design §3)
    if body.lines().count() + FOOTER_LINES > MAP_LINE_CAP {
        let overshoot = body.lines().count() + FOOTER_LINES - MAP_LINE_CAP;
        let collapsed = collapse_modules(&rows, overshoot);
        body = format!(
            "{header}{gate}\n{layout}{}{tests}\n{docs}\n",
            render_modules(&collapsed)
        );
    }

    let footer = format!(
        "{with_header} files with a header, {with_note} with a note, {without} without either — run /ratchet:map --deep for the rest.\n<!-- ratchet-map commit={full_sha} tree={tree_sha} -->\n"
    );
    let hints = wiring_hints(main_root);
    Ok(Generated {
        text: format!("{body}{footer}"),
        with_header,
        with_note,
        without,
        hints,
    })
}

pub fn write_map(main_root: &Path, generated: &Generated) -> Result<PathBuf, MapError> {
    let path = map_path(main_root);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| err(format!("could not create .ratchet/: {e}")))?;
    }
    fs::write(&path, &generated.text)
        .map_err(|e| err(format!("could not write {}: {e}", path.display())))?;
    Ok(path)
}

// --- freshness: shared by `map status` (Task 2) and the briefing line (Task 4) ----------------

pub fn recorded_commit(main_root: &Path) -> Option<String> {
    let text = fs::read_to_string(map_path(main_root)).ok()?;
    let last = text.lines().last()?;
    rest_after(last, "<!-- ratchet-map commit=")
}

fn rest_after(line: &str, prefix: &str) -> Option<String> {
    line.strip_prefix(prefix)?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    NoMap,
    Behind(u64),
    OtherBranch,
    Current,
    Unknown,
}

pub fn freshness(main_root: &Path) -> Freshness {
    let Some(recorded) = recorded_commit(main_root) else {
        return Freshness::NoMap;
    };
    let Some(head) = git_output(main_root, &["rev-parse", "HEAD"]) else {
        return Freshness::Unknown;
    };
    if recorded == head {
        return Freshness::Current;
    }
    let is_ancestor = Command::new("git")
        .args([
            "-C",
            &main_root.to_string_lossy(),
            "merge-base",
            "--is-ancestor",
            &recorded,
            "HEAD",
        ])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !is_ancestor {
        return Freshness::OtherBranch;
    }
    match git_output(
        main_root,
        &["rev-list", "--count", &format!("{recorded}..HEAD")],
    ) {
        Some(n) => n
            .trim()
            .parse()
            .map(Freshness::Behind)
            .unwrap_or(Freshness::Unknown),
        None => Freshness::Unknown,
    }
}

/// The line the briefing appends, or `None` when the map is current or freshness can't be
/// determined (design §6: a briefing never costs a session on a git failure).
pub fn briefing_line(main_root: &Path) -> Option<String> {
    match freshness(main_root) {
        Freshness::NoMap => Some("map: none — run /ratchet:map for the repo layout".to_string()),
        Freshness::Behind(n) => Some(format!("map: {n} commits behind — run /ratchet:map")),
        Freshness::OtherBranch => Some("map: from another branch — run /ratchet:map".to_string()),
        Freshness::Current | Freshness::Unknown => None,
    }
}

/// `ratchet map status`'s one line — always something, unlike `briefing_line`.
pub fn status_line(main_root: &Path) -> String {
    briefing_line(main_root).unwrap_or_else(|| "map: current".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(st.success());
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        git(d.path(), &["init", "-q", "-b", "main"]);
        d
    }

    #[test]
    fn cargo_manifest_detects_the_rust_gate() {
        let d = repo();
        fs::write(d.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        assert_eq!(
            detect_gate(d.path()),
            vec![
                "cargo fmt --all -- --check".to_string(),
                "cargo clippy --all-targets -- -D warnings".to_string(),
                "cargo test".to_string(),
            ]
        );
    }

    #[test]
    fn pyproject_detects_pytest_and_optional_ruff_mypy() {
        let d = repo();
        fs::write(
            d.path().join("pyproject.toml"),
            "[tool.ruff]\n[tool.mypy]\n",
        )
        .unwrap();
        assert_eq!(
            detect_gate(d.path()),
            vec![
                "pytest".to_string(),
                "ruff check".to_string(),
                "mypy".to_string()
            ]
        );
        fs::write(d.path().join("uv.lock"), "").unwrap();
        assert_eq!(detect_gate(d.path())[0], "uv run pytest");
    }

    #[test]
    fn package_json_detects_npm_scripts_via_lockfile() {
        let d = repo();
        fs::write(
            d.path().join("package.json"),
            r#"{"scripts":{"test":"x","lint":"y"}}"#,
        )
        .unwrap();
        assert_eq!(
            detect_gate(d.path()),
            vec!["npm run test".to_string(), "npm run lint".to_string()]
        );
        fs::write(d.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(
            detect_gate(d.path()),
            vec!["pnpm run test".to_string(), "pnpm run lint".to_string()]
        );
    }

    #[test]
    fn makefile_detects_test_and_lint_targets() {
        let d = repo();
        fs::write(
            d.path().join("Makefile"),
            "test:\n\techo t\nlint:\n\techo l\n",
        )
        .unwrap();
        assert_eq!(
            detect_gate(d.path()),
            vec!["make test".to_string(), "make lint".to_string()]
        );
    }

    #[test]
    fn go_mod_detects_go_test_and_vet() {
        let d = repo();
        fs::write(d.path().join("go.mod"), "module x\n").unwrap();
        assert_eq!(
            detect_gate(d.path()),
            vec!["go test ./...".to_string(), "go vet ./...".to_string()]
        );
    }

    #[test]
    fn map_gate_override_replaces_detection() {
        let d = repo();
        fs::write(d.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let cfg = MapSection {
            exclude: vec![],
            gate: vec!["make check".to_string()],
        };
        assert_eq!(gate_section(d.path(), &cfg), vec!["make check".to_string()]);
    }

    #[test]
    fn collapse_prefers_the_deepest_directory_first() {
        let rows: Vec<(PathBuf, String)> = vec![
            (PathBuf::from("a/x.rs"), "a/x.rs — one.".to_string()),
            (PathBuf::from("a/y.rs"), "a/y.rs — two.".to_string()),
            (PathBuf::from("a/b/z.rs"), "a/b/z.rs — three.".to_string()),
            (PathBuf::from("a/b/w.rs"), "a/b/w.rs — four.".to_string()),
        ];
        // Overshoot of 1: only the deepest directory (a/b, 2 components) collapses, not a/
        // (1 component) — even though both dirs have more than one file.
        let out = collapse_modules(&rows, 1);
        assert!(
            out.iter().any(|l| l.starts_with("a/b/ — 2 files")),
            "{out:?}"
        );
        assert!(out.contains(&"a/x.rs — one.".to_string()));
        assert!(out.contains(&"a/y.rs — two.".to_string()));
    }

    #[test]
    fn excluded_paths_match_a_star_glob() {
        assert!(is_excluded("vendor/blob.rs", &["vendor/*".to_string()]));
        assert!(!is_excluded("src/vendor.rs", &["vendor/*".to_string()]));
    }

    #[test]
    fn a_missing_notes_file_is_an_empty_map() {
        let d = tempfile::TempDir::new().unwrap();
        assert!(load_notes(d.path()).is_empty());
    }

    #[test]
    fn note_stores_the_repo_relative_form_even_from_an_absolute_target() {
        let d = repo();
        fs::write(d.path().join("x.rs"), "fn f() {}").unwrap();
        git(d.path(), &["add", "x.rs"]);
        git(d.path(), &["commit", "-q", "-m", "add x"]);
        note(d.path(), &d.path().join("x.rs"), "Does x.").unwrap();
        let notes = load_notes(d.path());
        assert_eq!(notes.get("x.rs"), Some(&"Does x.".to_string()));
    }

    #[test]
    fn missing_full_excludes_headered_and_noted_files() {
        let d = repo();
        fs::write(d.path().join("a.rs"), "//! Has a header\nfn a() {}").unwrap();
        fs::write(d.path().join("b.rs"), "fn b() {}").unwrap();
        fs::write(d.path().join("c.rs"), "fn c() {}").unwrap();
        git(d.path(), &["add", "a.rs", "b.rs", "c.rs"]);
        git(d.path(), &["commit", "-q", "-m", "add"]);
        note(d.path(), &d.path().join("c.rs"), "Has a note.").unwrap();
        let cfg = MapSection::default();
        let out = missing(d.path(), &cfg, true).unwrap();
        assert_eq!(out, vec!["b.rs".to_string()]);
    }
}
