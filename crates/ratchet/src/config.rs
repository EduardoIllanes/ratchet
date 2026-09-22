//! Configuration: the repo marker `ratchet.toml` and the machine file `~/.ratchet/config.toml`.
//! Pure parsing; no side effects beyond reading the named file.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

// Reachable once Task 9 (hooks::dispatch) and Task 10 (guardrails::cli) call repo::find_repo.
#[allow(dead_code)]
pub const MARKER: &str = "ratchet.toml";

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RepoConfig {
    pub repo: RepoSection,
    pub guardrails: GuardrailsSection,
    pub thresholds: Thresholds,
    pub map: MapSection,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RepoSection {
    pub name: Option<String>,
    pub default_branch: Option<String>,
    /// Relative to the main root. Default `.worktrees`.
    pub worktrees_dir: Option<String>,
}

/// Default for `[guardrails] big_read_lines`: a `Read` without a window, or a bare
/// `cat`/`head`/`tail`/`less`/`more`, over a regular file with more lines than this is blocked
/// by the built-in `big-read` rule.
pub const DEFAULT_BIG_READ_LINES: usize = 350;

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GuardrailsSection {
    /// Ids of rules disabled in this repo.
    pub off: Vec<String>,
    /// Path, relative to the main root, of a rules file with the built-in schema.
    pub extra: Option<String>,
    /// Line-count threshold for the built-in `big-read` rule.
    pub big_read_lines: usize,
    /// Rules declared inline, right in `ratchet.toml`, rather than in a separate `extra` file.
    /// Shape (unknown keys) is refused here by `deny_unknown_fields`; the semantic checks — a
    /// non-empty `message` that states the alternative, a `match` that compiles — are refused by
    /// `guardrails::rules::load_rule_set`, the same place the `extra` file's rules are validated
    /// (T-0012).
    pub rules: Vec<CustomRuleDecl>,
}

impl Default for GuardrailsSection {
    fn default() -> Self {
        Self {
            off: Vec::new(),
            extra: None,
            big_read_lines: DEFAULT_BIG_READ_LINES,
            rules: Vec::new(),
        }
    }
}

/// One `[[guardrails.rules]]` entry: a repo's own guardrail rule, declared inline in
/// `ratchet.toml` instead of a separate `extra` file. Always a `command`-kind rule (the same
/// matcher the built-in command rules use, over each command segment) — `kind`, `exempt` and
/// `requires` are not offered here; a repo that needs them uses the `extra` file schema instead.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CustomRuleDecl {
    pub name: String,
    /// A regex over each command segment, same as a built-in command rule's `pattern`.
    #[serde(rename = "match")]
    pub match_pattern: String,
    /// SHALL itself state the alternative (contain a form of "use" or "instead"); checked by
    /// `guardrails::rules::load_rule_set`, not here.
    pub message: String,
    /// Defaults to `["Bash", "PowerShell"]` when omitted.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MapSection {
    /// Directory globs (`*` matches any run of characters, including `/`) left out of the map
    /// entirely.
    pub exclude: Vec<String>,
    /// When non-empty, replaces gate detection entirely — printed verbatim, one per line.
    pub gate: Vec<String>,
}

/// Upper bound for a threshold read from `ratchet.toml`, in minutes: 100 years
/// (`60 * 24 * 365 * 100`). No real repo's `live_minutes`/`idle_minutes` would ever approach
/// this, and it is small enough that `Duration::minutes(v as i64)` — `v * 60_000` to reach
/// milliseconds — stays many orders of magnitude inside both `i64` and `chrono`'s own
/// `TimeDelta` range, so the cast in `sessions::state` can never overflow or panic no matter
/// what a hand-written marker file contains.
const MAX_THRESHOLD_MINUTES: u64 = 52_560_000;

fn clamp_minutes<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = u64::deserialize(deserializer)?;
    Ok(v.min(MAX_THRESHOLD_MINUTES))
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    #[serde(deserialize_with = "clamp_minutes")]
    pub live_minutes: u64,
    #[serde(deserialize_with = "clamp_minutes")]
    pub idle_minutes: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            live_minutes: 10,
            idle_minutes: 60,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PdfSettings {
    /// Command name (resolved on `PATH`) or path of the external extractor.
    pub extractor: String,
    /// Seconds allowed for the fast (`--no-ocr`) pass.
    pub timeout_s: u64,
    /// Seconds allowed for the OCR pass — far slower than the fast pass.
    pub ocr_timeout_s: u64,
    /// Below this many (trimmed) characters, the fast pass's result triggers an automatic OCR
    /// retry.
    pub ocr_min_chars: usize,
    /// Language passed to the extractor's `--ocr-language`.
    pub ocr_language: String,
    /// An input file larger than this is refused before any extraction.
    pub max_file_bytes: u64,
}

impl Default for PdfSettings {
    fn default() -> Self {
        Self {
            extractor: "liteparse".to_string(),
            timeout_s: 60,
            ocr_timeout_s: 600,
            ocr_min_chars: 200,
            ocr_language: "eng".to_string(),
            max_file_bytes: 200 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ModelWeights {
    /// Per million tokens, in whatever unit the owner likes. Unset fields default to zero.
    pub input: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    pub output: f64,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UsageSettings {
    /// Keyed by model prefix, e.g. `"claude-sonnet"`; matched by longest prefix
    /// (`usage::weights::matching`). Empty when `[usage.weights]` is absent from the file.
    pub weights: HashMap<String, ModelWeights>,
}

// Consumed by Task 6 (guardrails::rules::load_rule_set).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MachineConfig {
    pub guardrails: MachineGuardrails,
    pub pdf: PdfSettings,
    pub usage: UsageSettings,
}

// Field type of MachineConfig; consumed by Task 6 (guardrails::rules::load_rule_set).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MachineGuardrails {
    /// Path of a rules file; relative paths resolve against the state directory.
    pub extra: Option<String>,
}

// Consumed by Task 6 (guardrails::rules) and callers wired in Task 9 (hooks::dispatch).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for ConfigError {}

// Used by load_repo_config below; reachable once Task 9/10 call repo::find_repo.
#[allow(dead_code)]
pub fn parse_repo_config(text: &str, path: &Path) -> Result<RepoConfig, ConfigError> {
    toml::from_str(text).map_err(|e| ConfigError {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

// Used by repo::find_repo; reachable once Task 9 (hooks::dispatch) and Task 10 (guardrails::cli)
// call it.
#[allow(dead_code)]
pub fn load_repo_config(path: &Path) -> Result<RepoConfig, ConfigError> {
    let text = fs::read_to_string(path).map_err(|e| ConfigError {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    parse_repo_config(&text, path)
}

// Consumed by Task 9 (hooks::dispatch) and Task 10 (guardrails::cli).
#[allow(dead_code)]
pub fn ratchet_home(env: &HashMap<String, String>) -> PathBuf {
    if let Some(h) = env.get("RATCHET_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(h);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ratchet")
}

// Consumed by Task 6 (guardrails::rules::load_rule_set).
#[allow(dead_code)]
pub fn load_machine_config(home: &Path) -> Result<MachineConfig, ConfigError> {
    let path = home.join("config.toml");
    if !path.is_file() {
        return Ok(MachineConfig::default());
    }
    let text = fs::read_to_string(&path).map_err(|e| ConfigError {
        path: path.clone(),
        message: e.to_string(),
    })?;
    toml::from_str(&text).map_err(|e| ConfigError {
        path,
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn defaults_when_sections_are_missing() {
        let c = parse_repo_config("", Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.repo.worktrees_dir, None);
        assert!(c.guardrails.off.is_empty());
        assert_eq!(c.guardrails.big_read_lines, DEFAULT_BIG_READ_LINES);
        assert!(c.guardrails.rules.is_empty());
        assert_eq!(c.thresholds.live_minutes, 10);
        assert_eq!(c.thresholds.idle_minutes, 60);
    }

    #[test]
    fn parses_all_sections() {
        let text = "[repo]\nname = \"x\"\ndefault_branch = \"main\"\nworktrees_dir = \".wt\"\n[guardrails]\noff = [\"python-venv\"]\nextra = \"ratchet/g.toml\"\n[thresholds]\nlive_minutes = 3\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.repo.name.as_deref(), Some("x"));
        assert_eq!(c.repo.worktrees_dir.as_deref(), Some(".wt"));
        assert_eq!(c.guardrails.off, vec!["python-venv"]);
        assert_eq!(c.guardrails.extra.as_deref(), Some("ratchet/g.toml"));
        assert_eq!(
            c.guardrails.big_read_lines, DEFAULT_BIG_READ_LINES,
            "omitted from this repo's [guardrails] table: falls back to the default"
        );
        assert_eq!(c.thresholds.live_minutes, 3);
    }

    #[test]
    fn big_read_lines_is_overridable_and_defaults_when_absent() {
        let text = "[guardrails]\nbig_read_lines = 1000\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.guardrails.big_read_lines, 1000);
        let c = parse_repo_config("[guardrails]\noff = []\n", Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.guardrails.big_read_lines, DEFAULT_BIG_READ_LINES);
    }

    #[test]
    fn invalid_toml_names_the_file() {
        let err = parse_repo_config("[repo\nx = ", Path::new("C:/r/ratchet.toml")).unwrap_err();
        assert!(err.to_string().contains("ratchet.toml"), "{err}");
    }

    #[test]
    fn ordinary_thresholds_pass_through_untouched() {
        let text = "[thresholds]\nlive_minutes = 15\nidle_minutes = 45\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.thresholds.live_minutes, 15);
        assert_eq!(c.thresholds.idle_minutes, 45);
    }

    #[test]
    fn live_minutes_zero_is_kept_as_is() {
        let text = "[thresholds]\nlive_minutes = 0\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.thresholds.live_minutes, 0);
    }

    #[test]
    fn an_absurd_threshold_is_clamped_to_the_maximum() {
        let text =
            "[thresholds]\nlive_minutes = 200000000000000000\nidle_minutes = 999999999999999999\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.thresholds.live_minutes, MAX_THRESHOLD_MINUTES);
        assert_eq!(c.thresholds.idle_minutes, MAX_THRESHOLD_MINUTES);
        // The clamped value must never make `Duration::minutes(v as i64)` panic.
        let _ = chrono::Duration::minutes(c.thresholds.live_minutes as i64);
        let _ = chrono::Duration::minutes(c.thresholds.idle_minutes as i64);
    }

    #[test]
    fn map_section_default_and_parsed() {
        let c = parse_repo_config("", Path::new("ratchet.toml")).unwrap();
        assert!(c.map.exclude.is_empty());
        assert!(c.map.gate.is_empty());

        let text = "[map]\nexclude = [\"vendor/*\"]\ngate = [\"make check\"]\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.map.exclude, vec!["vendor/*"]);
        assert_eq!(c.map.gate, vec!["make check"]);
    }

    #[test]
    fn unknown_key_is_an_error() {
        let err =
            parse_repo_config("[guardrails]\nofff = []\n", Path::new("ratchet.toml")).unwrap_err();
        assert!(err.message.contains("offf"), "{}", err.message);
    }

    #[test]
    fn inline_guardrail_rules_parse_with_and_without_tools() {
        let text = "[[guardrails.rules]]\nname = \"no-curl\"\nmatch = '^curl'\nmessage = \"Use the fetch script instead.\"\ntools = [\"Bash\"]\n\n[[guardrails.rules]]\nname = \"no-wget\"\nmatch = '^wget'\nmessage = \"Use the fetch script instead.\"\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.guardrails.rules.len(), 2);
        assert_eq!(c.guardrails.rules[0].name, "no-curl");
        assert_eq!(c.guardrails.rules[0].match_pattern, "^curl");
        assert_eq!(
            c.guardrails.rules[0].tools.as_deref(),
            Some(&["Bash".to_string()][..])
        );
        assert_eq!(c.guardrails.rules[1].name, "no-wget");
        assert_eq!(
            c.guardrails.rules[1].tools, None,
            "omitted tools stays None; the default set is applied when the rule is converted"
        );
    }

    #[test]
    fn inline_guardrail_rule_with_an_unknown_key_is_an_error() {
        let text =
            "[[guardrails.rules]]\nname = \"x\"\nmatch = 'y'\nmessage = \"use z instead\"\nbogus = 1\n";
        let err = parse_repo_config(text, Path::new("ratchet.toml")).unwrap_err();
        assert!(err.message.contains("bogus"), "{}", err.message);
    }

    #[test]
    fn home_from_env_or_default() {
        let mut env = std::collections::HashMap::new();
        env.insert("RATCHET_HOME".to_string(), "C:/tmp/rh".to_string());
        assert_eq!(ratchet_home(&env), PathBuf::from("C:/tmp/rh"));
        env.clear();
        assert!(ratchet_home(&env).ends_with(".ratchet"));
    }

    #[test]
    fn machine_config_missing_is_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let c = load_machine_config(dir.path()).unwrap();
        assert_eq!(c.guardrails.extra, None);
    }

    #[test]
    fn pdf_settings_default_and_parsed() {
        let dir = tempfile::TempDir::new().unwrap();
        let c = load_machine_config(dir.path()).unwrap();
        assert_eq!(c.pdf.extractor, "liteparse");
        assert_eq!(c.pdf.ocr_min_chars, 200);

        std::fs::write(
            dir.path().join("config.toml"),
            "[pdf]\nextractor = \"C:/x/liteparse.cmd\"\ntimeout_s = 5\n",
        )
        .unwrap();
        let c = load_machine_config(dir.path()).unwrap();
        assert_eq!(c.pdf.extractor, "C:/x/liteparse.cmd");
        assert_eq!(c.pdf.timeout_s, 5);
        assert_eq!(c.pdf.ocr_timeout_s, 600, "unset fields keep their default");
    }

    // Proves an unknown key under `[pdf]` fails the same documented way as every other table in
    // this file (an `Err(ConfigError)` naming the bad field) rather than panicking — the caller
    // in the pre-tool hot path (guardrails::rules::load_rule_set, Task 6) already turns that Err
    // into a non-fatal, logged outcome, same as it does for a bad `[guardrails]` table.
    #[test]
    fn pdf_unknown_key_is_an_error_like_other_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("config.toml"), "[pdf]\nbogus = 1\n").unwrap();
        let err = load_machine_config(dir.path()).unwrap_err();
        assert!(err.message.contains("bogus"), "{}", err.message);
    }

    #[test]
    fn usage_settings_default_is_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let c = load_machine_config(dir.path()).unwrap();
        assert!(c.usage.weights.is_empty());
    }

    #[test]
    fn usage_weights_parse_a_quoted_prefix_table() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[usage.weights.\"claude-sonnet\"]\ninput = 3.0\ncache_write = 3.75\ncache_read = 0.3\noutput = 15.0\n",
        )
        .unwrap();
        let c = load_machine_config(dir.path()).unwrap();
        let w = c.usage.weights.get("claude-sonnet").unwrap();
        assert_eq!(w.input, 3.0);
        assert_eq!(w.output, 15.0);
    }

    #[test]
    fn usage_weights_unknown_key_is_an_error_like_other_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[usage.weights.\"x\"]\nbogus = 1\n",
        )
        .unwrap();
        let err = load_machine_config(dir.path()).unwrap_err();
        assert!(err.message.contains("bogus"), "{}", err.message);
    }
}
