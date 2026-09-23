//! Guardrail rules: schema, built-ins, extension files and merging. Pure; the only I/O is
//! reading the extra files named by the machine and repo configs.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::{self, ConfigError};
use crate::log;
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

/// A whole guardrail layer -- the repo's own marker (`ratchet.toml`), or a machine/repo `extra`
/// rules file -- that could not be read or parsed at all (T-0017), as opposed to a single rule
/// within an otherwise-valid file (`DroppedRule`). The repo stays opted in: built-ins and every
/// other layer that DID parse still apply; this only records which layer was skipped and why, for
/// `ratchet.log` and the session-start briefing.
#[derive(Debug, Clone, PartialEq)]
pub struct BrokenLayer {
    /// The file that could not be read or parsed: the repo marker, or the resolved path of a
    /// machine/repo `extra` file.
    pub file: PathBuf,
    /// A human reason (a read error or a TOML parse error), without the file path -- the caller
    /// already has it via `file`.
    pub reason: String,
}

/// A rule left out of the merged set because it failed validation on its own (T-0015): a
/// builtin-id collision, `tools = []`, a pattern that does not compile as a regex, an unknown
/// key, or any other per-rule problem `custom_rule`/`parse_rules` catches. The built-ins and
/// every other valid rule -- from either the inline `[[guardrails.rules]]` list or an `extra`
/// file -- still apply; only this one rule is missing from `RuleSet::rules`.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedRule {
    /// The rule's declared id/name, or a placeholder when the entry was too malformed to name
    /// itself (no `id` key at all in an `extra` file's `[[rules]]` table).
    pub id: String,
    /// The file the rule came from: the repo marker (`ratchet.toml`) for an inline rule, or the
    /// resolved path of a machine/repo `extra` file.
    pub file: PathBuf,
    /// A human reason, without the file path (the caller already has it via `file`) and without
    /// repeating the id (the caller already has it via `id`).
    pub reason: String,
}

// Consumed by Task 7 (guardrails::eval) and Task 9 (hooks::dispatch).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    pub off: Vec<String>,
    /// Rules left out this load because they failed validation on their own (T-0015); empty
    /// when every declared rule was valid.
    pub dropped: Vec<DroppedRule>,
    /// Whole layers (the repo marker, or a machine/repo `extra` file) that could not be read or
    /// parsed at all this load (T-0017); empty when every layer parsed.
    pub broken: Vec<BrokenLayer>,
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
    parse_rules(BUILTIN_TOML, Path::new("<builtin>"), "builtin")
        .expect("builtin rules are valid")
        .0
}

/// Checks a parsed `Rule` for what `deny_unknown_fields` cannot catch on its own: `pattern`/
/// `exempt` compile as a regex, and a kind that needs a pattern has one. The id and file are
/// added by the caller, which already has both.
fn validate_pattern_rule(r: &Rule) -> Result<(), String> {
    for (label, pat) in [("pattern", &r.pattern), ("exempt", &r.exempt)] {
        if let Some(p) = pat {
            if let Err(e) = fancy_regex::Regex::new(&format!("(?im){p}")) {
                return Err(format!("invalid {label} regex: {e}"));
            }
        }
    }
    if r.kind != Kind::MainTree && r.kind != Kind::BigRead && r.pattern.is_none() {
        return Err(format!(
            "`pattern` is required for kind {}",
            r.kind.as_str()
        ));
    }
    Ok(())
}

/// Parses a rules file (built-in, machine or repo `extra`, same schema): a syntactically valid
/// TOML document whose only top-level key is `rules`, an array of tables.
///
/// A structurally broken file -- unparseable TOML, or a top-level shape other than `{ rules =
/// [...] }` -- is `Err` here; the caller (`load_rule_set`) records that as a `BrokenLayer` and
/// skips the whole file rather than propagating the error further (T-0017), same as a
/// structurally broken `ratchet.toml` itself. Within a structurally valid `rules` array, each
/// entry is parsed and validated **on its own**: one entry with an unknown key, a bad kind, a
/// missing required field or an uncompilable regex is left out (`DroppedRule`) rather than
/// failing the whole file, so a sibling entry -- and every built-in -- still applies.
pub fn parse_rules(
    text: &str,
    path: &Path,
    source: &str,
) -> Result<(Vec<Rule>, Vec<DroppedRule>), ConfigError> {
    let err = |message: String| ConfigError {
        path: path.to_path_buf(),
        message,
    };
    let doc: toml::Value = toml::from_str(text).map_err(|e| err(e.to_string()))?;
    let table = match &doc {
        toml::Value::Table(t) => t,
        _ => return Err(err("expected a table with a `rules` array".to_string())),
    };
    for key in table.keys() {
        if key != "rules" {
            return Err(err(format!(
                "unknown top-level key `{key}` (only `rules` is recognized)"
            )));
        }
    }
    let items = match table.get("rules") {
        None => Vec::new(),
        Some(toml::Value::Array(items)) => items.clone(),
        Some(_) => return Err(err("`rules` must be an array of tables".to_string())),
    };
    let mut rules = Vec::new();
    let mut dropped = Vec::new();
    for item in items {
        let label = item
            .get("id")
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        match Rule::deserialize(item) {
            Ok(mut r) => match validate_pattern_rule(&r) {
                Ok(()) => {
                    r.source = source.to_string();
                    rules.push(r);
                }
                Err(reason) => dropped.push(DroppedRule {
                    id: label.unwrap_or(r.id),
                    file: path.to_path_buf(),
                    reason,
                }),
            },
            Err(e) => dropped.push(DroppedRule {
                id: label.unwrap_or_else(|| "<rule with no id>".to_string()),
                file: path.to_path_buf(),
                reason: e.to_string(),
            }),
        }
    }
    Ok((rules, dropped))
}

fn load_rules_file(
    path: &Path,
    source: &str,
) -> Result<(Vec<Rule>, Vec<DroppedRule>), ConfigError> {
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
/// what `deny_unknown_fields` cannot: a `name` that does not collide with a built-in id, a
/// non-empty `message` that states the alternative, a `tools` list that is not explicitly
/// empty, and a `match` pattern that compiles. `alternative` is left empty — the declared
/// `message` already carries it — so `Violation::render` prints the message alone instead of
/// duplicating it.
fn custom_rule(
    decl: &config::CustomRuleDecl,
    marker_path: &Path,
    builtin_ids: &[String],
) -> Result<Rule, ConfigError> {
    let err = |message: String| ConfigError {
        path: marker_path.to_path_buf(),
        message,
    };
    // An inline rule is always `Kind::Command` (see `CustomRuleDecl`), so reusing a built-in's
    // id would not extend it — `merge()`'s same-id-replaces semantics would swap the built-in
    // out for a command-only rule in its place. For a non-command built-in (`env-files`,
    // `main-tree`, `big-read`) that silently guts the protection: it stops matching anything a
    // Write/Edit ever populates in `tool_input.command`, with no warning. Refuse it instead, the
    // same way an `extra`-file collision is left legal (that schema lets the override state its
    // own `kind` deliberately, so it is not the footgun this is).
    if builtin_ids.iter().any(|id| id == &decl.name) {
        return Err(err(format!(
            "rule `{}`: this name is a built-in guardrail id; an inline `[[guardrails.rules]]` \
             rule is command-only and would silently replace the built-in instead of extending \
             it. Rename this rule, or disable the built-in with `off = [\"{}\"]` under \
             [guardrails] in ratchet.toml",
            decl.name, decl.name
        )));
    }
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
    if let Some(tools) = &decl.tools {
        if tools.is_empty() {
            return Err(err(format!(
                "rule `{}`: `tools` must not be empty; omit it for the default (Bash, \
                 PowerShell), or list at least one tool",
                decl.name
            )));
        }
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

/// `custom_rule`'s errors read as a standalone message, `` rule `{id}`: ... ``, for when they
/// are the whole story (the strict CLI path, `load_rule_set_strict`). A `DroppedRule` already
/// names the id in its own field, so this strips the duplicate prefix before it becomes the
/// `reason` -- otherwise a briefing/log line would read "rule `env-files` dropped: rule
/// `env-files`: ...".
fn without_rule_prefix(id: &str, message: &str) -> String {
    message
        .strip_prefix(&format!("rule `{id}`: "))
        .unwrap_or(message)
        .to_string()
}

/// built-ins, then the machine file (`~/.ratchet/config.toml` → `guardrails.extra`), then the
/// repo file (`ratchet.toml` → `guardrails.extra`, relative to the main root). A rule that fails
/// validation on its own -- inline or from an `extra` file -- is left out rather than failing
/// this whole load (T-0015): the built-ins and every other valid rule still apply.
///
/// Nothing here is fatal (T-0017): the repo's own marker, the machine config, and any `extra`
/// file may each fail to be read or parsed on their own, and each such failure is recorded as a
/// `BrokenLayer` rather than aborting the load -- built-ins, and every other layer that DID parse
/// (machine or repo `extra`, inline rules), still apply. A repo whose marker itself failed to
/// parse (`repo.marker_error`, set by `Repo::find_repo`) already carries `RepoConfig::default()`,
/// so its `extra`/inline/`off` sections are simply absent here, same as a repo with none declared.
/// Every dropped rule and every broken layer gets one `ratchet.log` entry naming it, its file and
/// why.
// Consumed by Task 9 (hooks::dispatch).
#[allow(dead_code)]
pub fn load_rule_set(home: &Path, repo: Option<&Repo>) -> RuleSet {
    let mut rules = builtin_rules();
    // Fixed at the built-in set (not whatever `rules` grows into below): an inline rule is
    // refused for colliding with one of these five ids specifically, regardless of what a
    // machine or repo `extra` file layers on top.
    let builtin_ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
    let mut dropped: Vec<DroppedRule> = Vec::new();
    let mut broken: Vec<BrokenLayer> = Vec::new();
    if let Some(repo) = repo {
        if let Some(e) = &repo.marker_error {
            broken.push(BrokenLayer {
                file: e.path.clone(),
                reason: e.message.clone(),
            });
        }
    }
    let machine = match config::load_machine_config(home) {
        Ok(m) => m,
        Err(e) => {
            broken.push(BrokenLayer {
                file: e.path,
                reason: e.message,
            });
            config::MachineConfig::default()
        }
    };
    if let Some(extra) = machine.guardrails.extra {
        let path = resolve(&extra, home);
        match load_rules_file(&path, "machine") {
            Ok((extra_rules, extra_dropped)) => {
                rules = merge(rules, extra_rules);
                dropped.extend(extra_dropped);
            }
            Err(e) => broken.push(BrokenLayer {
                file: e.path,
                reason: e.message,
            }),
        }
    }
    let mut off = Vec::new();
    if let Some(repo) = repo {
        if let Some(extra) = &repo.config.guardrails.extra {
            let path = resolve(extra, &repo.main_root);
            match load_rules_file(&path, "repo") {
                Ok((extra_rules, extra_dropped)) => {
                    rules = merge(rules, extra_rules);
                    dropped.extend(extra_dropped);
                }
                Err(e) => broken.push(BrokenLayer {
                    file: e.path,
                    reason: e.message,
                }),
            }
        }
        // Inline `[[guardrails.rules]]` rules evaluate after everything above — appended last,
        // so an earlier rule (built-in or `extra`) with an overlapping pattern still wins. A
        // name that collides with a built-in id is not an overlap to resolve this way; it is
        // dropped by `custom_rule` before it ever reaches `merge`.
        if !repo.config.guardrails.rules.is_empty() {
            let marker_path = repo.main_root.join(config::MARKER);
            let mut custom = Vec::with_capacity(repo.config.guardrails.rules.len());
            for decl in &repo.config.guardrails.rules {
                match custom_rule(decl, &marker_path, &builtin_ids) {
                    Ok(r) => custom.push(r),
                    Err(e) => dropped.push(DroppedRule {
                        id: decl.name.clone(),
                        file: marker_path.clone(),
                        reason: without_rule_prefix(&decl.name, &e.message),
                    }),
                }
            }
            rules = merge(rules, custom);
        }
        off = repo.config.guardrails.off.clone();
    }
    for d in &dropped {
        log::append(
            home,
            &format!(
                "guardrail rule `{}` dropped from {}: {}",
                d.id,
                d.file.display(),
                d.reason
            ),
        );
    }
    for b in &broken {
        log::append(
            home,
            &format!(
                "guardrail config {} could not be read: {}",
                b.file.display(),
                b.reason
            ),
        );
    }
    RuleSet {
        rules,
        off,
        dropped,
        broken,
    }
}

/// Same inputs as `load_rule_set`, but strict: any rule that would otherwise be dropped -- an
/// inline collision, `tools = []`, a bad regex, an unknown key -- and any layer that would
/// otherwise be skipped as broken -- the repo marker, the machine config, or an `extra` file that
/// cannot be read or parsed (T-0017) -- is promoted to a hard `Err` instead. `ratchet guardrails
/// list`/`test` use this: a diagnostic over the guardrail config that silently tolerated the very
/// thing it exists to catch would defeat its own purpose. The hook path
/// (`hooks::dispatch::pre_tool`/`post_tool`/`session_start`) uses the resilient `load_rule_set`
/// instead, so one bad rule or one broken layer never disables every guardrail for the session.
pub fn load_rule_set_strict(home: &Path, repo: Option<&Repo>) -> Result<RuleSet, ConfigError> {
    let set = load_rule_set(home, repo);
    if let Some(b) = set.broken.first() {
        return Err(ConfigError {
            path: b.file.clone(),
            message: b.reason.clone(),
        });
    }
    if let Some(d) = set.dropped.first() {
        return Err(ConfigError {
            path: d.file.clone(),
            message: format!("rule `{}`: {}", d.id, d.reason),
        });
    }
    Ok(set)
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

    /// The five built-in ids, for tests exercising `custom_rule`'s collision check without
    /// caring about the machine/repo `extra` layers `load_rule_set` adds on top.
    fn builtin_ids() -> Vec<String> {
        super::builtin_rules().into_iter().map(|r| r.id).collect()
    }

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
        let err = custom_rule(&decl, Path::new("ratchet.toml"), &builtin_ids()).unwrap_err();
        assert!(err.message.contains('x'), "{}", err.message);
        assert!(err.message.contains("empty"), "{}", err.message);
    }

    #[test]
    fn custom_rule_colliding_with_a_command_kind_builtin_id_is_refused() {
        let decl = config::CustomRuleDecl {
            name: "git-destructive".into(),
            match_pattern: "never-matches-anything".into(),
            message: "use x instead".into(),
            tools: None,
        };
        let err = custom_rule(&decl, Path::new("ratchet.toml"), &builtin_ids()).unwrap_err();
        assert!(err.to_string().contains("ratchet.toml"), "{err}");
        assert!(err.message.contains("git-destructive"), "{}", err.message);
        assert!(err.message.contains("off"), "{}", err.message);
    }

    #[test]
    fn custom_rule_colliding_with_a_non_command_builtin_id_is_refused() {
        for id in ["env-files", "main-tree", "big-read"] {
            let decl = config::CustomRuleDecl {
                name: id.into(),
                match_pattern: "never-matches-anything".into(),
                message: "use x instead".into(),
                tools: None,
            };
            let err = custom_rule(&decl, Path::new("ratchet.toml"), &builtin_ids()).unwrap_err();
            assert!(err.message.contains(id), "{}", err.message);
        }
    }

    #[test]
    fn custom_rule_with_empty_tools_is_refused() {
        let decl = config::CustomRuleDecl {
            name: "no-curl".into(),
            match_pattern: "^curl".into(),
            message: "Use the fetch script instead.".into(),
            tools: Some(vec![]),
        };
        let err = custom_rule(&decl, Path::new("ratchet.toml"), &builtin_ids()).unwrap_err();
        assert!(err.message.contains("no-curl"), "{}", err.message);
        assert!(err.message.contains("tools"), "{}", err.message);
    }

    #[test]
    fn custom_rule_with_no_stated_alternative_is_refused() {
        let decl = config::CustomRuleDecl {
            name: "no-curl".into(),
            match_pattern: "^curl".into(),
            message: "Curl is not allowed here.".into(),
            tools: None,
        };
        let err = custom_rule(&decl, Path::new("ratchet.toml"), &builtin_ids()).unwrap_err();
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
        let err = custom_rule(&decl, Path::new("g.toml"), &builtin_ids()).unwrap_err();
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
        let rule = custom_rule(&decl, Path::new("ratchet.toml"), &builtin_ids()).unwrap();
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
        let rule = custom_rule(&decl, Path::new("ratchet.toml"), &builtin_ids()).unwrap();
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
    fn parse_drops_a_bad_regex_naming_the_rule_and_file_instead_of_failing_the_file() {
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = '('\nmessage = \"m\"\nalternative = \"a\"\n\
             [[rules]]\nid = \"y\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'z'\nmessage = \"m\"\nalternative = \"a\"\n";
        let (rules, dropped) = parse_rules(text, Path::new("g.toml"), "repo").unwrap();
        // The sibling, valid rule still loads even though `x` was dropped.
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert_eq!(rules[0].id, "y");
        assert_eq!(dropped.len(), 1, "{dropped:?}");
        assert_eq!(dropped[0].id, "x");
        assert_eq!(dropped[0].file, Path::new("g.toml"));
        assert!(dropped[0].reason.contains("regex"), "{}", dropped[0].reason);
    }

    #[test]
    fn parse_drops_an_unknown_kind_and_a_missing_message_each_on_their_own() {
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"weird\"\nmessage = \"m\"\nalternative = \"a\"\n";
        let (rules, dropped) = parse_rules(text, Path::new("g.toml"), "repo").unwrap();
        assert!(rules.is_empty());
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].id, "x");

        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'a'\nalternative = \"a\"\n";
        let (rules, dropped) = parse_rules(text, Path::new("g.toml"), "repo").unwrap();
        assert!(rules.is_empty());
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].id, "x");
    }

    #[test]
    fn parse_drops_an_unknown_key_naming_the_rule() {
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'a'\nmessage = \"m\"\nalternative = \"a\"\nbogus = 1\n";
        let (rules, dropped) = parse_rules(text, Path::new("g.toml"), "repo").unwrap();
        assert!(rules.is_empty());
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].id, "x");
        assert!(dropped[0].reason.contains("bogus"), "{}", dropped[0].reason);
    }

    #[test]
    fn parse_rejects_a_structurally_broken_extra_file() {
        // Unparseable TOML, and a top-level shape other than `rules = [...]`, stay fatal --
        // out of scope for T-0015 (same reasoning as a structurally broken `ratchet.toml`).
        let err = parse_rules("not valid toml [[[", Path::new("g.toml"), "repo").unwrap_err();
        assert!(err.to_string().contains("g.toml"), "{err}");
        let err =
            parse_rules("rules = \"not an array\"\n", Path::new("g.toml"), "repo").unwrap_err();
        assert!(err.message.contains("array"), "{}", err.message);
        let err =
            parse_rules("unexpected_top_level = 1\n", Path::new("g.toml"), "repo").unwrap_err();
        assert!(
            err.message.contains("unexpected_top_level"),
            "{}",
            err.message
        );
    }

    #[test]
    fn merge_replaces_same_id_in_place_and_appends_new() {
        let base = builtin_rules();
        let (extra, dropped) = parse_rules(
            "[[rules]]\nid = \"git-destructive\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'x'\nmessage = \"new\"\nalternative = \"a\"\n[[rules]]\nid = \"custom\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'y'\nmessage = \"c\"\nalternative = \"a\"\n",
            Path::new("g.toml"),
            "machine",
        )
        .unwrap();
        assert!(dropped.is_empty(), "{dropped:?}");
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
            dropped: Vec::new(),
            broken: Vec::new(),
        };
        let ids: Vec<&str> = set.active().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["git-destructive", "env-files", "main-tree", "big-read"]
        );
    }

    /// Minimal `Repo` for `load_rule_set` tests below -- only the fields it reads matter.
    fn test_repo(
        main_root: &Path,
        config: config::RepoConfig,
        marker_error: Option<ConfigError>,
    ) -> Repo {
        Repo {
            main_root: main_root.to_path_buf(),
            checkout_root: main_root.to_path_buf(),
            name: "r".to_string(),
            worktrees_dir: main_root.join(".worktrees"),
            config,
            marker_error,
        }
    }

    // T-0017: a marker that failed to parse (`repo.marker_error`) is recorded as a broken layer,
    // not propagated as `Err` -- the repo's `config` is already `RepoConfig::default()` by the
    // time it reaches here (`Repo::find_repo` sets it), so only the built-ins load.
    #[test]
    fn load_rule_set_carries_the_repo_marker_error_as_a_broken_layer_and_falls_back_to_builtins() {
        let dir = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let marker_path = dir.path().join(config::MARKER);
        let repo = test_repo(
            dir.path(),
            config::RepoConfig::default(),
            Some(ConfigError {
                path: marker_path.clone(),
                message: "bad toml".into(),
            }),
        );
        let set = load_rule_set(home.path(), Some(&repo));
        assert_eq!(set.broken.len(), 1, "{:?}", set.broken);
        assert_eq!(set.broken[0].file, marker_path);
        assert!(set.dropped.is_empty(), "{:?}", set.dropped);
        let ids: Vec<&str> = set.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "python-venv",
                "git-destructive",
                "env-files",
                "main-tree",
                "big-read"
            ],
            "builtins only: a broken marker's config is RepoConfig::default(), no extra/inline"
        );
    }

    // T-0017: a broken repo `extra` file is skipped (recorded as a broken layer), while the
    // built-ins and an inline `[[guardrails.rules]]` rule declared in the (valid) marker itself
    // still load.
    #[test]
    fn load_rule_set_skips_a_broken_repo_extra_file_but_keeps_builtins_and_inline_rules() {
        let dir = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("guardrails.toml"), "[[rules\nnot valid").unwrap();
        let mut cfg = config::RepoConfig::default();
        cfg.guardrails.extra = Some("guardrails.toml".to_string());
        cfg.guardrails.rules = vec![config::CustomRuleDecl {
            name: "no-curl".into(),
            match_pattern: "^curl".into(),
            message: "Use the fetch script instead.".into(),
            tools: None,
        }];
        let repo = test_repo(dir.path(), cfg, None);
        let set = load_rule_set(home.path(), Some(&repo));
        assert_eq!(set.broken.len(), 1, "{:?}", set.broken);
        assert_eq!(set.broken[0].file, dir.path().join("guardrails.toml"));
        let ids: Vec<&str> = set.rules.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"env-files"), "{ids:?}");
        assert!(ids.contains(&"no-curl"), "{ids:?}");
    }

    // T-0017: `load_rule_set_strict` (the `guardrails list`/`test` diagnostic path) promotes a
    // broken layer to a hard `Err`, unlike the resilient hook path above.
    #[test]
    fn load_rule_set_strict_promotes_a_broken_layer_to_a_hard_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let marker_path = dir.path().join(config::MARKER);
        let repo = test_repo(
            dir.path(),
            config::RepoConfig::default(),
            Some(ConfigError {
                path: marker_path.clone(),
                message: "bad toml".into(),
            }),
        );
        let err = load_rule_set_strict(home.path(), Some(&repo)).unwrap_err();
        assert_eq!(err.path, marker_path);
        assert!(err.message.contains("bad toml"), "{}", err.message);
    }
}
