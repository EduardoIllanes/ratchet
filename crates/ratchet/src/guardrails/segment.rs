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

/// One whitespace-delimited token starting at `chars[start]` (the caller is expected to have
/// already skipped any leading whitespace), honouring double quotes, single quotes (fully
/// literal — no escapes recognised inside), and a backslash escape outside quotes (the
/// backslash is dropped, the escaped character kept, matching how a shell hands the argument to
/// the program). A quoted piece glued to a bare piece with no separating whitespace (`"a"b`,
/// `a'b'`) joins into one token, matching shell word-splitting; the returned token has its
/// quoting stripped. Returns `None` when `start` is at or past the end of `chars`.
pub fn scan_token(chars: &[char], start: usize) -> Option<(String, usize)> {
    let n = chars.len();
    if start >= n {
        return None;
    }
    let mut buf = String::new();
    let mut quote: Option<char> = None;
    let mut i = start;
    while i < n {
        let ch = chars[i];
        if let Some(q) = quote {
            if ch == '\\' && q == '"' && i + 1 < n {
                buf.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if ch == q {
                quote = None;
            } else {
                buf.push(ch);
            }
            i += 1;
            continue;
        }
        if ch.is_whitespace() {
            break;
        }
        if ch == '\\' && i + 1 < n {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        buf.push(ch);
        i += 1;
    }
    Some((buf, i))
}

/// Split a segment into whitespace-separated tokens the way a shell would, honouring the same
/// quoting `scan_token` does. Unlike `str::split_whitespace`, a quoted piece containing a space
/// survives as one token with its quotes stripped: `cp a "my dir/file"` tokenizes to
/// `["cp", "a", "my dir/file"]`, not four pieces. Shared by every guardrail that needs to read a
/// command's arguments (`main_tree::shape_targets`, `big_read::reader_command_targets`) so the
/// quote-awareness lives in one place.
pub fn tokenize(segment: &str) -> Vec<String> {
    let chars: Vec<char> = segment.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        let Some((tok, next)) = scan_token(&chars, i) else {
            break;
        };
        out.push(tok);
        i = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{scan_token, segments, tokenize};

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

    #[test]
    fn tokenize_keeps_a_quoted_space_as_one_token() {
        assert_eq!(
            tokenize(r#"cp /tmp/x "src/my dir/file.rs""#),
            vec!["cp", "/tmp/x", "src/my dir/file.rs"]
        );
    }

    #[test]
    fn tokenize_strips_quotes_and_joins_adjacent_quoted_and_bare_pieces() {
        assert_eq!(tokenize(r#"a"b c"d"#), vec!["ab cd"]);
        assert_eq!(tokenize("'it''s' fine"), vec!["its", "fine"]);
    }

    #[test]
    fn tokenize_honours_backslash_escape_outside_quotes() {
        assert_eq!(tokenize(r"my\ file.rs plain"), vec!["my file.rs", "plain"]);
    }

    #[test]
    fn tokenize_single_quotes_do_not_recognise_backslash_escapes() {
        assert_eq!(tokenize(r"'a\ b'"), vec![r"a\ b"]);
    }

    #[test]
    fn scan_token_returns_the_index_just_past_the_token() {
        let chars: Vec<char> = r#">"path" tail"#.chars().collect();
        let (tok, next) = scan_token(&chars, 1).unwrap();
        assert_eq!(tok, "path");
        assert_eq!(&chars[next..], [' ', 't', 'a', 'i', 'l']);
    }

    #[test]
    fn scan_token_none_past_the_end() {
        let chars: Vec<char> = "abc".chars().collect();
        assert_eq!(scan_token(&chars, 3), None);
    }
}
