//! Local PDF text extraction through an external extractor, behind a seam so no test ever runs
//! the real one. There is no network code anywhere in this file.
//!
//! The extractor is a CLI named in machine config (`liteparse` by default), not a Rust crate:
//! pure-Rust PDF readers see only an embedded text layer, have no OCR, and give up on damaged
//! real-world files. The price is one external install
//! (`npm i -g @llamaindex/liteparse`); `ratchet pdf` says so by name when it is missing.

use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::PdfSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// The call is refused before, or instead of, running the extractor.
    Refused(String),
    /// The extractor ran (or tried to) and did not produce a usable answer.
    Unavailable(String),
}

impl PdfError {
    pub fn message(&self) -> &str {
        match self {
            PdfError::Refused(m) | PdfError::Unavailable(m) => m,
        }
    }
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for PdfError {}

/// One invocation of the extractor.
#[derive(Debug, Clone)]
pub struct Run {
    pub text: String,
    // Populated by every `Extractor::run` but not read by `ratchet pdf` (Task 3's CLI header
    // reports only `pages`/`ocr`/`chars`/`sink`, per the contract's pinned header lines); part
    // of the public API, kept for a future header/log line rather than dropped.
    #[allow(dead_code)]
    pub ms: u64,
    /// Page count, when the extractor reported one.
    // Same as `ms`: populated, part of the public `Run` API, not read by the CLI today.
    #[allow(dead_code)]
    pub pages: Option<u32>,
}

pub trait Extractor {
    // `Liteparse::name`/`Liteparse::version` are implemented (the trait requires them) but
    // `ratchet pdf` never calls either through the `dyn Extractor` — the header names the
    // configured command via `PdfSettings.extractor`, not the running extractor's own report of
    // itself. Kept on the trait as part of its public surface (e.g. for future diagnostics).
    #[allow(dead_code)]
    fn name(&self) -> String;
    #[allow(dead_code)]
    fn version(&self) -> String;
    /// One pass. `no_ocr` picks the fast pass; `out` is the scratch file the extractor writes.
    fn run(
        &self,
        pdf: &Path,
        out: &Path,
        no_ocr: bool,
        pages: Option<&str>,
        timeout_s: u64,
    ) -> Result<Run, PdfError>;
}

/// Refuses a missing file, a file over the cap, or one whose first bytes are not the PDF magic
/// number (`%PDF-`) — before any extractor is resolved or invoked.
pub fn check_input_file(path: &Path, max_bytes: u64) -> Result<(), PdfError> {
    let meta = fs::metadata(path)
        .map_err(|_| PdfError::Refused(format!("file not found: {}", path.display())))?;
    if !meta.is_file() {
        return Err(PdfError::Refused(format!(
            "file not found: {}",
            path.display()
        )));
    }
    if meta.len() > max_bytes {
        return Err(PdfError::Refused(format!(
            "input file larger than the cap: {} ({} bytes, cap {} bytes)",
            path.display(),
            meta.len(),
            max_bytes
        )));
    }
    use std::io::Read;
    let mut f = File::open(path)
        .map_err(|e| PdfError::Refused(format!("file not found: {} ({e})", path.display())))?;
    let mut magic = [0u8; 5];
    let n = f.read(&mut magic).unwrap_or(0);
    if n < 5 || &magic != b"%PDF-" {
        return Err(PdfError::Refused(format!(
            "not a pdf file: {}",
            path.display()
        )));
    }
    Ok(())
}

/// `--pages` accepts a comma list of `N` or `N-M` (positive integers, `N <= M`). Anything else
/// is refused before any request. There is no separate "sink component" step: a validated
/// range is already filesystem-safe (digits, `-`, `,`), so the CLI uses it in the file name
/// verbatim.
pub fn validate_pages(spec: &str) -> Result<(), PdfError> {
    let bad = || PdfError::Refused(format!("invalid page range: {spec:?}"));
    if spec.trim().is_empty() {
        return Err(bad());
    }
    for part in spec.split(',') {
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (parse_pos(a), parse_pos(b));
                match (a, b) {
                    (Some(a), Some(b)) if a <= b => {}
                    _ => return Err(bad()),
                }
            }
            None => {
                if parse_pos(part).is_none() {
                    return Err(bad());
                }
            }
        }
    }
    Ok(())
}

fn parse_pos(s: &str) -> Option<u32> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok().filter(|&n| n >= 1)
}

/// A bare name is looked up on `PATH` (honouring `PATHEXT` on Windows); a name containing a
/// path separator is taken as a path and simply has to exist — this is the branch every
/// scenario test uses (Ruling GP-R1/GP-R2), pointing straight at a fake script.
pub fn resolve(name: &str) -> Option<String> {
    if name.contains('/') || name.contains('\\') {
        return Path::new(name).is_file().then(|| name.to_string());
    }
    let exts: Vec<String> = match std::env::var("PATHEXT") {
        Ok(v) if !v.is_empty() => v.split(';').map(|e| e.to_lowercase()).collect(),
        _ => vec![String::new()],
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
        let bare = dir.join(name);
        if bare.is_file() {
            return Some(bare.to_string_lossy().to_string());
        }
    }
    None
}

/// The extractor this run uses, or a refusal naming the install command. Resolved once, before
/// the input file is touched for extraction (spec §4.6: "refused before any work").
pub fn extractor_for(s: &PdfSettings) -> Result<Box<dyn Extractor>, PdfError> {
    match resolve(&s.extractor) {
        None => Err(missing(&s.extractor)),
        Some(cmd) => Ok(Box::new(Liteparse {
            cmd,
            ocr_language: s.ocr_language.clone(),
        })),
    }
}

fn missing(name: &str) -> PdfError {
    PdfError::Refused(format!(
        "no extractor for PDF: {name:?} was not found. PDFs need the liteparse CLI; install it \
         with: npm i -g @llamaindex/liteparse"
    ))
}

/// The OCR policy of one document. Returns the run that produced the text, the mode
/// (`skipped` / `fallback` / `forced` / `disabled`), and the fast pass when it was run and then
/// discarded (only in the `fallback` case).
pub fn extract_text(
    ex: &dyn Extractor,
    pdf: &Path,
    out: &Path,
    ocr: Option<bool>,
    pages: Option<&str>,
    s: &PdfSettings,
) -> Result<(Run, &'static str, Option<Run>), PdfError> {
    if ocr == Some(true) {
        let run = ex.run(pdf, out, false, pages, s.ocr_timeout_s)?;
        return Ok((run, "forced", None));
    }
    let fast = ex.run(pdf, out, true, pages, s.timeout_s)?;
    if ocr == Some(false) {
        return Ok((fast, "disabled", None));
    }
    if fast.text.trim().chars().count() >= s.ocr_min_chars {
        return Ok((fast, "skipped", None));
    }
    // Nearly empty: a scanned page. The OCR pass is authoritative — if it fails, the call
    // fails; it never falls back to the short text.
    let ocr_run = ex.run(pdf, out, false, pages, s.ocr_timeout_s)?;
    Ok((ocr_run, "fallback", Some(fast)))
}

/// Two shapes seen in the wild: `(83 pages)` in the extractor's progress line, and `pages: 83`.
/// Best-effort only — nothing in this plan requires it to succeed.
pub fn pages_from_output(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    if let Some(at) = lower.find(" pages") {
        let head: String = lower[..at]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !head.is_empty() {
            return head.chars().rev().collect::<String>().parse().ok();
        }
    }
    for marker in ["pages:", "pages "] {
        if let Some(at) = lower.find(marker) {
            let tail: String = lower[at + marker.len()..]
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !tail.is_empty() {
                return tail.parse().ok();
            }
        }
    }
    None
}

// --- the real extractor -------------------------------------------------------------------

pub struct Liteparse {
    cmd: String,
    ocr_language: String,
}

impl Extractor for Liteparse {
    fn name(&self) -> String {
        self.cmd.clone()
    }

    fn version(&self) -> String {
        let mut c = Command::new(&self.cmd);
        c.arg("-V");
        match run_capturing(c, 10, std::env::temp_dir().as_path()) {
            Ok((_, text)) => first_line(&text).unwrap_or_else(|| "unknown".to_string()),
            Err(_) => "unknown".to_string(),
        }
    }

    fn run(
        &self,
        pdf: &Path,
        out: &Path,
        no_ocr: bool,
        pages: Option<&str>,
        timeout_s: u64,
    ) -> Result<Run, PdfError> {
        let mut c = Command::new(&self.cmd);
        c.arg("parse")
            .arg(pdf)
            .arg("--format")
            .arg("text")
            .arg("-o")
            .arg(out);
        if no_ocr {
            // `-q` only on the fast pass: the OCR pass's progress line is where the page count
            // usually shows up.
            c.arg("--no-ocr").arg("-q");
        } else {
            c.arg("--ocr-language").arg(&self.ocr_language);
        }
        if let Some(p) = pages {
            c.arg("--target-pages").arg(p);
        }
        let log_dir = out.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(log_dir).ok();
        let started = Instant::now();
        let (success, output) = run_capturing(c, timeout_s, log_dir)?;
        let ms = started.elapsed().as_millis() as u64;
        if !success {
            return Err(PdfError::Unavailable(format!(
                "extractor failed: {}",
                first_line(&output).unwrap_or_else(|| "non-zero exit".to_string())
            )));
        }
        let text = fs::read_to_string(out).map_err(|e| {
            PdfError::Unavailable(format!(
                "extractor produced no output file: {} ({e})",
                out.display()
            ))
        })?;
        Ok(Run {
            text,
            ms,
            pages: pages_from_output(&output),
        })
    }
}

/// Run a child with a timeout, its output redirected to files (not pipes: reading a pipe while
/// polling for exit deadlocks once the child fills the buffer, and extractors are chatty).
/// `std::process` has no timeout of its own, so the wait is a 50 ms poll — precise enough for a
/// limit measured in tens of seconds to minutes.
fn run_capturing(
    mut cmd: Command,
    timeout_s: u64,
    log_dir: &Path,
) -> Result<(bool, String), PdfError> {
    // Per-process names (GP-P7 / B-1): a fixed `extractor.out.log`/`extractor.err.log` let two
    // concurrent `ratchet pdf` runs read and overwrite each other's captured output. Removed
    // again below on every return path (success, failure, timeout, or a spawn/IO error) so the
    // sink directory never accumulates one pair per run; nothing here is user-visible beyond the
    // one-line `error: extractor failed: …` message, which is built from `text`/`output` while
    // the files still exist, not read back from disk afterwards.
    let pid = std::process::id();
    let out_path = log_dir.join(format!("extractor-{pid}.out.log"));
    let err_path = log_dir.join(format!("extractor-{pid}.err.log"));
    let cleanup = || {
        let _ = fs::remove_file(&out_path);
        let _ = fs::remove_file(&err_path);
    };
    let out_file = match File::create(&out_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::remove_file(&out_path);
            return Err(PdfError::Unavailable(format!("extractor failed: {e}")));
        }
    };
    let err_file = match File::create(&err_path) {
        Ok(f) => f,
        Err(e) => {
            cleanup();
            return Err(PdfError::Unavailable(format!("extractor failed: {e}")));
        }
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            cleanup();
            return Err(PdfError::Unavailable(format!("extractor failed: {e}")));
        }
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    loop {
        match child.try_wait() {
            Err(e) => {
                cleanup();
                return Err(PdfError::Unavailable(format!("extractor failed: {e}")));
            }
            Ok(Some(status)) => {
                let text = format!(
                    "{}\n{}",
                    fs::read_to_string(&out_path).unwrap_or_default(),
                    fs::read_to_string(&err_path).unwrap_or_default()
                );
                cleanup();
                return Ok((status.success(), text));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    cleanup();
                    return Err(PdfError::Unavailable(format!(
                        "extractor failed: timeout after {timeout_s}s"
                    )));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        no_ocr_text: String,
        ocr_text: String,
        fail_ocr: bool,
        calls: RefCell<Vec<String>>,
    }

    impl Extractor for Fake {
        fn name(&self) -> String {
            "fake".into()
        }
        fn version(&self) -> String {
            "fake 0.0".into()
        }
        fn run(
            &self,
            _pdf: &Path,
            out: &Path,
            no_ocr: bool,
            pages: Option<&str>,
            _timeout_s: u64,
        ) -> Result<Run, PdfError> {
            self.calls.borrow_mut().push(format!(
                "{} pages={}",
                if no_ocr { "noocr" } else { "ocr" },
                pages.unwrap_or("-")
            ));
            if !no_ocr && self.fail_ocr {
                return Err(PdfError::Unavailable(
                    "extractor failed: ocr exploded".into(),
                ));
            }
            let text = if no_ocr {
                &self.no_ocr_text
            } else {
                &self.ocr_text
            };
            fs::create_dir_all(out.parent().unwrap()).unwrap();
            fs::write(out, text).unwrap();
            Ok(Run {
                text: text.clone(),
                ms: 1,
                pages: Some(7),
            })
        }
    }

    fn settings() -> PdfSettings {
        PdfSettings {
            ocr_min_chars: 10,
            ..Default::default()
        }
    }

    fn fake(no_ocr_text: &str, ocr_text: &str, fail_ocr: bool) -> Fake {
        Fake {
            no_ocr_text: no_ocr_text.into(),
            ocr_text: ocr_text.into(),
            fail_ocr,
            calls: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn a_text_layer_skips_ocr() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = fake("a long enough text layer", "OCR", false);
        let (run, mode, first) = extract_text(
            &f,
            &dir.path().join("a.pdf"),
            &dir.path().join("a.txt"),
            None,
            None,
            &settings(),
        )
        .unwrap();
        assert_eq!(mode, "skipped");
        assert!(run.text.contains("text layer"));
        assert!(first.is_none());
        assert_eq!(f.calls.borrow().len(), 1);
    }

    #[test]
    fn two_extract_text_calls_with_different_out_paths_do_not_share_state() {
        // Regression pin for GP-P7 / B-1. The actual bug was two *processes* sharing one
        // hardcoded scratch path (fixed in `cli/pdf_cmd.rs` and the log names in this file's
        // `run_capturing`); a two-process race is not reproducible as a `#[test]`, so this
        // instead pins the invariant one layer down, at the `pdf.rs` API these processes call:
        // two calls with distinct `out` paths must never mix their text, on the returned `Run`
        // or on disk.
        let dir = tempfile::TempDir::new().unwrap();
        let out_a = dir.path().join("run-a.txt");
        let out_b = dir.path().join("run-b.txt");
        let a = fake(
            "payload for run A, plenty of characters to skip the ocr retry",
            "unused",
            false,
        );
        let b = fake(
            "payload for run B, also plenty of characters to skip the ocr retry",
            "unused",
            false,
        );

        let (run_a, mode_a, _) = extract_text(
            &a,
            &dir.path().join("a.pdf"),
            &out_a,
            None,
            None,
            &settings(),
        )
        .unwrap();
        let (run_b, mode_b, _) = extract_text(
            &b,
            &dir.path().join("b.pdf"),
            &out_b,
            None,
            None,
            &settings(),
        )
        .unwrap();

        assert_eq!(mode_a, "skipped");
        assert_eq!(mode_b, "skipped");
        assert!(run_a.text.contains("run A"), "{}", run_a.text);
        assert!(run_b.text.contains("run B"), "{}", run_b.text);
        assert_ne!(run_a.text, run_b.text);
        assert_eq!(fs::read_to_string(&out_a).unwrap(), run_a.text);
        assert_eq!(fs::read_to_string(&out_b).unwrap(), run_b.text);
    }

    #[test]
    fn nearly_empty_text_falls_back_to_ocr() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = fake("  ", "text that only OCR could read", false);
        let (run, mode, first) = extract_text(
            &f,
            &dir.path().join("a.pdf"),
            &dir.path().join("a.txt"),
            None,
            None,
            &settings(),
        )
        .unwrap();
        assert_eq!(mode, "fallback");
        assert!(run.text.contains("only OCR"));
        assert_eq!(first.unwrap().text.trim(), "");
        assert_eq!(
            *f.calls.borrow(),
            vec!["noocr pages=-".to_string(), "ocr pages=-".to_string()]
        );
    }

    #[test]
    fn forced_and_disabled_ocr_modes() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = fake("short", "OCR text", false);
        let (_, mode, _) = extract_text(
            &f,
            &dir.path().join("a.pdf"),
            &dir.path().join("a.txt"),
            Some(true),
            Some("1-3"),
            &settings(),
        )
        .unwrap();
        assert_eq!(mode, "forced");
        assert_eq!(*f.calls.borrow(), vec!["ocr pages=1-3".to_string()]);

        let g = fake("short", "OCR text", false);
        let (run, mode, _) = extract_text(
            &g,
            &dir.path().join("a.pdf"),
            &dir.path().join("a.txt"),
            Some(false),
            None,
            &settings(),
        )
        .unwrap();
        assert_eq!(mode, "disabled");
        assert_eq!(run.text, "short");
        assert_eq!(g.calls.borrow().len(), 1);
    }

    #[test]
    fn a_failing_ocr_pass_is_a_refusal_not_a_silent_short_answer() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = fake("  ", "never used", true);
        let e = extract_text(
            &f,
            &dir.path().join("a.pdf"),
            &dir.path().join("a.txt"),
            None,
            None,
            &settings(),
        )
        .unwrap_err();
        assert!(e.message().contains("extractor failed"), "{e}");
    }

    #[test]
    fn page_counts_are_read_from_either_shape_of_output() {
        assert_eq!(
            pages_from_output("[liteparse] extract: 1427.8ms (83 pages)"),
            Some(83)
        );
        assert_eq!(pages_from_output("pages: 12"), Some(12));
        assert_eq!(pages_from_output("nothing here"), None);
    }

    #[test]
    fn a_missing_extractor_is_a_named_refusal() {
        let s = PdfSettings {
            extractor: "C:/definitely/not/here/liteparse".to_string(),
            ..Default::default()
        };
        // `Result::unwrap_err` needs `T: Debug`, and `Box<dyn Extractor>` isn't (the trait has
        // no `Debug` supertrait), so this matches by hand instead of the brief's `.unwrap_err()`.
        let e = match extractor_for(&s) {
            Err(e) => e,
            Ok(_) => panic!("expected a refusal for a missing extractor"),
        };
        assert!(e.message().contains("no extractor"), "{e}");
        assert!(
            e.message().contains("npm i -g @llamaindex/liteparse"),
            "{e}"
        );
    }

    #[test]
    fn check_input_file_rejects_missing_non_pdf_and_oversize() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("nope.pdf");
        assert!(check_input_file(&missing, 1_000_000)
            .unwrap_err()
            .message()
            .contains("not found"));

        let not_pdf = dir.path().join("x.pdf");
        fs::write(&not_pdf, b"hello world").unwrap();
        assert!(check_input_file(&not_pdf, 1_000_000)
            .unwrap_err()
            .message()
            .contains("not a pdf file"));

        let real = dir.path().join("real.pdf");
        fs::write(&real, b"%PDF-1.4\nsome bytes").unwrap();
        assert!(check_input_file(&real, 1_000_000).is_ok());
        assert!(check_input_file(&real, 3)
            .unwrap_err()
            .message()
            .contains("larger than the cap"));
    }

    #[test]
    fn validate_pages_accepts_ranges_and_singles_rejects_the_rest() {
        assert!(validate_pages("1-8,12").is_ok());
        assert!(validate_pages("12").is_ok());
        assert!(validate_pages("0").is_err());
        assert!(validate_pages("8-1").is_err());
        assert!(validate_pages("").is_err());
        assert!(validate_pages("abc").is_err());
        assert!(validate_pages("1,,2").is_err());
    }
}
