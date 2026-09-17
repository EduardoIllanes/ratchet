//! Split a shell command into segments on `;`, `&&`, `||`, `|` and newlines, only outside
//! quotes. We do not de-quote (shlex would fight Windows paths); we only need to know where to
//! cut, so we scan the raw text carrying the quote state.

#[allow(dead_code)] // consumed by Task 7 (guardrails::eval)
pub fn segments(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    let n = chars.len();
    while i < n {
        let ch = chars[i];
        match quote {
            Some(q) => {
                if ch == '\\' && q == '"' && i + 1 < n {
                    buf.push(ch);
                    buf.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if ch == q {
                    quote = None;
                }
                buf.push(ch);
            }
            None => {
                if ch == '\\' && i + 1 < n {
                    buf.push(ch);
                    buf.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                    buf.push(ch);
                } else if ch == ';' || ch == '\n' || ch == '|' {
                    out.push(std::mem::take(&mut buf));
                    if ch == '|' && i + 1 < n && chars[i + 1] == '|' {
                        i += 1;
                    }
                } else if ch == '&' && i + 1 < n && chars[i + 1] == '&' {
                    out.push(std::mem::take(&mut buf));
                    i += 1;
                } else {
                    buf.push(ch);
                }
            }
        }
        i += 1;
    }
    out.push(buf);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::segments;

    #[test]
    fn respects_quotes() {
        assert_eq!(
            segments(r#"uv run ratchet task new -c "a; mypy b" && echo done"#),
            vec![r#"uv run ratchet task new -c "a; mypy b""#, "echo done"]
        );
    }

    #[test]
    fn splits_on_every_operator_and_newline() {
        assert_eq!(
            segments("uv run a; mypy b | ruff c || pytest d\npython e"),
            vec!["uv run a", "mypy b", "ruff c", "pytest d", "python e"]
        );
    }

    #[test]
    fn unclosed_quote_keeps_the_rest_whole() {
        let cmd = r#"uv run ratchet task note T-1 "left it; pytest green"#;
        assert_eq!(segments(cmd), vec![cmd]);
    }

    #[test]
    fn escaped_semicolon_outside_quotes_does_not_split() {
        assert_eq!(segments(r"echo a\; python b"), vec![r"echo a\; python b"]);
    }

    #[test]
    fn windows_backslashes_inside_quotes_are_fine() {
        assert_eq!(
            segments(r#"uv run python "C:\repos\x\s.py; pytest later""#),
            vec![r#"uv run python "C:\repos\x\s.py; pytest later""#]
        );
    }

    #[test]
    fn single_ampersand_does_not_split() {
        assert_eq!(segments("sleep 1 & echo bg"), vec!["sleep 1 & echo bg"]);
    }
}
