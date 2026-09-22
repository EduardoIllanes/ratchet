//! Guardrail rules: schema, built-ins, extension files and merging. Pure; the only I/O is
//! reading the extra files named by the machine and repo configs.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::{self, ConfigError};
use crate::repo::Repo;

pub const BUILTIN_TOML: &str = include_str!("builtin.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Command,
    FilePath,
    Content,
    MainTree,
    BigRead,
}

impl Kind {
    // Consumed by Task 7 (guardrails::eval) error/message formatting; used locally in parse_rules.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Command => "command",
            Kind::FilePath => "file_path",
            Kind::Content => "content",
            Kind::MainTree => "main_tree",
            Kind::BigRead => "big_read",
        }
    }
}

// Fields tools/message/alternative/requires are read by Task 7 (guardrails::eval); id/kind/
// pattern/exempt/source are read within this module.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub tools: Vec<String>,
    pub kind: Kind,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub exempt: Option<String>,
    #[serde(default)]
    pub requires: Option<String>,
    pub message: String,
    pub alternative: String,
    /// `builtin`, `machine` or `repo`. Not read from the file.
    #[serde(skip)]
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    #[serde(default)]
    rules: Vec<Rule>,
}

// Consumed by Task 7 (guardrails::eval) and Task 9 (hooks::dispatch).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    pub off: Vec<String>,
}

// Methods consumed by Task 7/9.
#[allow(dead_code)]
impl RuleSet {
    pub fn active(&self) -> impl Iterator<Item = &Rule> {
        self.rules
            .iter()
            .filter(move |r| !self.off.iter().any(|o| o == &r.id))
    }
    pub fn is_off(&self, id: &str) -> bool {
        self.off.iter().any(|o| o == id)
    }
}

pub fn builtin_rules() -> Vec<Rule> {
    parse_rules(BUILTIN_TOML, Path::new("<builtin>"), "builtin").expect("builtin rules are valid")
}

pub fn parse_rules(text: &str, path: &Path, source: &str) -> Result<Vec<Rule>, ConfigError> {
    let file: RuleFile = toml::from_str(text).map_err(|e| ConfigError {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    let mut rules = file.rules;
    for r in &mut rules {
        r.source = source.to_string();
        for (label, pat) in [("pattern", &r.pattern), ("exempt", &r.exempt)] {
            if let Some(p) = pat {
                fancy_regex::Regex::new(&format!("(?im){p}")).map_err(|e| ConfigError {
                    path: path.to_path_buf(),
                    message: format!("rule `{}`: invalid {label} regex: {e}", r.id),
                })?;
            }
        }
        if r.kind != Kind::MainTree && r.kind != Kind::BigRead && r.pattern.is_none() {
            return Err(ConfigError {
                path: path.to_path_buf(),
                message: format!(
                    "rule `{}`: `pattern` is required for kind {}",
                    r.id,
                    r.kind.as_str()
                ),
            });
        }
    }
    Ok(rules)
}

fn load_rules_file(path: &Path, source: &str) -> Result<Vec<Rule>, ConfigError> {
    let text = fs::read_to_string(path).map_err(|e| ConfigError {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    parse_rules(&text, path, source)
}

/// Tools a repo-declared inline rule (`[[guardrails.rules]]`) applies to when `tools` is
/// omitted — the same set the built-in command rules `python-venv`/`git-destructive` use.
const DEFAULT_CUSTOM_RULE_TOOLS: [&str; 2] = ["Bash", "PowerShell"];

/// A rule's message must itself tell the agent what to do differently (AGENTS.md, Rule 1). A
/// built-in rule states it in a separate `alternative` field (see
/// `every_builtin_rule_states_its_alternative` below); an inline `[[guardrails.rules]]` entry
/// has only `message`, so this checks the message carries the alternative itself — a
/// deliberately loose token match, not free-form NLP, good enough to catch a message that
/// forgot the alternative entirely.
fn states_alternative(message: &str) -> bool {
    message.split(|c: char| !c.is_alphanumeric()).any(|w| {
        matches!(
            w.to_lowercase().as_str(),
            "use" | "uses" | "used" | "using" | "instead"
        )
    })
}

/// Converts one `[[guardrails.rules]]` declaration into the internal `Rule` shape, validating
/// what `deny_unknown_fields` cannot: a non-empty `message` that states the alternative, and a
/// `match` pattern that compiles. `alternative` is left empty — the declared `message` already
/// carries it — so `Violation::render` prints the message alone instead of duplicating it.
fn custom_rule(decl: &config::CustomRuleDecl, marker_path: &Path) -> Result<Rule, ConfigError> {
    let err = |message: String| ConfigError {
        path: marker_path.to_path_buf(),
        message,
    };
    if decl.message.trim().is_empty() {
        return Err(err(format!(
            "rule `{}`: message must not be empty",
            decl.name
        )));
    }
    if !states_alternative(&decl.message) {
        return Err(err(format!(
            "rule `{}`: message must state the alternative (e.g. contain \"use\" or \"instead\")",
            decl.name
        )));
    }
    fancy_regex::Regex::new(&format!("(?im){}", decl.match_pattern))
        .map_err(|e| err(format!("rule `{}`: invalid match regex: {e}", decl.name)))?;
    Ok(Rule {
        id: decl.name.clone(),
        tools: decl.tools.clone().unwrap_or_else(|| {
            DEFAULT_CUSTOM_RULE_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect()
        }),
        kind: Kind::Command,
        pattern: Some(decl.match_pattern.clone()),
        exempt: None,
        requires: None,
        message: decl.message.clone(),
        alternative: String::new(),
        source: "repo".to_string(),
    })
}

/// Later rules with an existing id replace the earlier rule in place; new ids append.
pub fn merge(mut base: Vec<Rule>, extra: Vec<Rule>) -> Vec<Rule> {
    for r in extra {
        match base.iter_mut().find(|b| b.id == r.id) {
            Some(slot) => *slot = r,
            None => base.push(r),
        }
    }
    base
}

/// built-ins, then the machine file (`~/.ratchet/config.toml` → `guardrails.extra`), then the
/// repo file (`ratchet.toml` → `guardrails.extra`, relative to the main root).
// Consumed by Task 9 (hooks::dispatch).
#[allow(dead_code)]
pub fn load_rule_set(home: &Path, repo: Option<&Repo>) -> Result<RuleSet, ConfigError> {
    let mut rules = builtin_rules();
    let machine = config::load_machine_config(home)?;
    if let Some(extra) = machine.guardrails.extra {
        let path = resolve(&extra, home);
        rules = merge(rules, load_rules_file(&path, "machine")?);
    }
    let mut off = Vec::new();
    if let Some(repo) = repo {
        if let Some(extra) = &repo.config.guardrails.extra {
            let path = resolve(extra, &repo.main_root);
            rules = merge(rules, load_rules_file(&path, "repo")?);
        }
        // Inline `[[guardrails.rules]]` rules evaluate after everything above — appended last,
        // so an earlier rule (built-in or `extra`) with an overlapping pattern still wins.
        if !repo.config.guardrails.rules.is_empty() {
            let marker_path = repo.main_root.join(config::MARKER);
            let mut custom = Vec::with_capacity(repo.config.guardrails.rules.len());
            for decl in &repo.config.guardrails.rules {
                custom.push(custom_rule(decl, &marker_path)?);
            }
            rules = merge(rules, custom);
        }
        off = repo.config.guardrails.off.clone();
    }
    Ok(RuleSet { rules, off })
}

fn resolve(p: &str, base: &Path) -> PathBuf {
    let path = PathBuf::from(p);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // T-0012, Rule 1 audit: every built-in block message names the alternative the agent should
    // use, so it corrects itself in one attempt. Builtin.toml already separates `message` and
    // `alternative`, and the schema requires both — this test is the one place that check is
    // actually exercised for all five, so a future rule added without an alternative fails here
    // instead of shipping silently.
    #[test]
    fn every_builtin_rule_states_its_alternative() {
        for r in builtin_rules() {
            assert!(
                !r.alternative.trim().is_empty(),
                "rule `{}` has no alternative",
                r.id
            );
        }
    }

    #[test]
    fn custom_rule_with_an_empty_message_is_refused() {
        let decl = config::CustomRuleDecl {
            name: "x".into(),
            match_pattern: "y".into(),
            message: "   ".into(),
            tools: None,
        };
        let err = custom_rule(&decl, Path::new("ratchet.toml")).unwrap_err();
        assert!(err.message.contains('x'), "{}", err.message);
        assert!(err.message.contains("empty"), "{}", err.message);
    }

    #[test]
    fn custom_rule_with_no_stated_alternative_is_refused() {
        let decl = config::CustomRuleDecl {
            name: "no-curl".into(),
            match_pattern: "^curl".into(),
            message: "Curl is not allowed here.".into(),
            tools: None,
        };
        let err = custom_rule(&decl, Path::new("ratchet.toml")).unwrap_err();
        assert!(err.message.contains("no-curl"), "{}", err.message);
        assert!(err.message.contains("alternative"), "{}", err.message);
    }

    #[test]
    fn custom_rule_with_a_bad_regex_is_refused() {
        let decl = config::CustomRuleDecl {
            name: "x".into(),
            match_pattern: "(".into(),
            message: "Use y instead.".into(),
            tools: None,
        };
        let err = custom_rule(&decl, Path::new("g.toml")).unwrap_err();
        assert!(err.to_string().contains("g.toml"), "{err}");
        assert!(
            err.message.contains("invalid match regex"),
            "{}",
            err.message
        );
    }

    #[test]
    fn custom_rule_converts_with_default_tools_and_no_duplicated_alternative() {
        let decl = config::CustomRuleDecl {
            name: "no-curl".into(),
            match_pattern: "^curl".into(),
            message: "Use the repo's fetch script instead.".into(),
            tools: None,
        };
        let rule = custom_rule(&decl, Path::new("ratchet.toml")).unwrap();
        assert_eq!(rule.id, "no-curl");
        assert_eq!(
            rule.tools,
            vec!["Bash".to_string(), "PowerShell".to_string()]
        );
        assert_eq!(rule.kind, Kind::Command);
        assert_eq!(rule.message, "Use the repo's fetch script instead.");
        assert_eq!(rule.alternative, "");
        assert_eq!(rule.source, "repo");
    }

    #[test]
    fn custom_rule_keeps_an_explicit_tools_list() {
        let decl = config::CustomRuleDecl {
            name: "no-curl".into(),
            match_pattern: "^curl".into(),
            message: "Use the repo's fetch script instead.".into(),
            tools: Some(vec!["Bash".to_string()]),
        };
        let rule = custom_rule(&decl, Path::new("ratchet.toml")).unwrap();
        assert_eq!(rule.tools, vec!["Bash".to_string()]);
    }

    #[test]
    fn builtins_have_the_five_ids_in_order() {
        let ids: Vec<String> = builtin_rules().into_iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            vec![
                "python-venv",
                "git-destructive",
                "env-files",
                "main-tree",
                "big-read"
            ]
        );
        assert!(builtin_rules().iter().all(|r| r.source == "builtin"));
    }

    #[test]
    fn parse_rejects_bad_regex_with_the_file_named() {
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = '('\nmessage = \"m\"\nalternative = \"a\"\n";
        let err = parse_rules(text, Path::new("g.toml"), "repo").unwrap_err();
        assert!(err.to_string().contains("g.toml"), "{err}");
        assert!(err.message.contains("x"), "{}", err.message);
    }

    #[test]
    fn parse_rejects_unknown_kind_and_missing_message() {
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"weird\"\nmessage = \"m\"\nalternative = \"a\"\n";
        assert!(parse_rules(text, Path::new("g.toml"), "repo").is_err());
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'a'\nalternative = \"a\"\n";
        assert!(parse_rules(text, Path::new("g.toml"), "repo").is_err());
    }

    #[test]
    fn merge_replaces_same_id_in_place_and_appends_new() {
        let base = builtin_rules();
        let extra = parse_rules(
            "[[rules]]\nid = \"git-destructive\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'x'\nmessage = \"new\"\nalternative = \"a\"\n[[rules]]\nid = \"custom\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'y'\nmessage = \"c\"\nalternative = \"a\"\n",
            Path::new("g.toml"),
            "machine",
        )
        .unwrap();
        let merged = merge(base, extra);
        let ids: Vec<&str> = merged.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "python-venv",
                "git-destructive",
                "env-files",
                "main-tree",
                "big-read",
                "custom"
            ]
        );
        assert_eq!(merged[1].message, "new");
        assert_eq!(merged[1].source, "machine");
    }

    #[test]
    fn rule_set_active_excludes_off() {
        let set = RuleSet {
            rules: builtin_rules(),
            off: vec!["python-venv".into()],
        };
        let ids: Vec<&str> = set.active().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["git-destructive", "env-files", "main-tree", "big-read"]
        );
    }
}
