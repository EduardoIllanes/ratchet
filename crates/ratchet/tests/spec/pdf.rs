//! Scenario tests for `openspec/specs/pdf/spec.md`. Every `#### Scenario` there has exactly one
//! test here, named by slug. No test runs the real `liteparse` — see `support::PdfBox`.

use std::path::Path;

use crate::support::{code, pdf, stderr, stdout, PdfBox};

/// `ocr_min_chars = 20` keeps every test's short fixture text a deliberate choice (well above
/// or well below 20) instead of an accident of the default (200).
fn cfg(extra: &str) -> String {
    format!("timeout_s = 2\nocr_timeout_s = 2\nocr_min_chars = 20\n{extra}")
}

#[test]
fn pdf__missing_file_is_refused() {
    let pb = PdfBox::new();
    pb.install_fake_liteparse();
    pb.config(&pb.fake_extractor_path(), &cfg(""));
    let out = pdf(&pb, &["nope.pdf"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("not found"), "{}", stderr(&out));
    assert!(pb.calls().is_empty());
}

#[test]
fn pdf__non_pdf_file_is_refused() {
    let pb = PdfBox::new();
    pb.install_fake_liteparse();
    pb.config(&pb.fake_extractor_path(), &cfg(""));
    let f = pb.write_non_pdf("x.pdf");
    let out = pdf(&pb, &[f.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("not a pdf file"), "{}", stderr(&out));
    assert!(pb.calls().is_empty());
}

#[test]
fn pdf__missing_extractor_refuses_before_any_extraction() {
    let pb = PdfBox::new();
    // No install_fake_liteparse(): the configured path simply does not exist.
    pb.config(Path::new("C:/definitely/not/here/liteparse.cmd"), &cfg(""));
    let f = pb.write_pdf("a.pdf");
    let out = pdf(&pb, &[f.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let e = stderr(&out);
    assert!(e.contains("no extractor"), "{e}");
    assert!(e.contains("npm i -g @llamaindex/liteparse"), "{e}");
    assert!(pb.calls().is_empty());
}

#[test]
fn pdf__page_range_narrows_the_extraction_and_the_sink_file_name() {
    let pb = PdfBox::new();
    pb.install_fake_liteparse();
    pb.config(&pb.fake_extractor_path(), &cfg(""));
    pb.set_text(
        "noocr",
        Some("2-3"),
        "pages two and three, plenty of text to skip the ocr retry",
    );
    let f = pb.write_pdf("report.pdf");
    let out = pdf(&pb, &[f.to_str().unwrap(), "--pages", "2-3"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let s = stdout(&out);
    assert!(s.contains("pages: 2-3"), "{s}");
    let sink_line = s.lines().find(|l| l.starts_with("sink:")).unwrap();
    let sink_path = sink_line.trim_start_matches("sink:").trim();
    assert!(sink_path.contains("report-p2-3.txt"), "{sink_path}");
    assert!(!sink_path.contains("-ocr"), "{sink_path}");
    assert_eq!(
        std::fs::read_to_string(sink_path).unwrap(),
        "pages two and three, plenty of text to skip the ocr retry"
    );
    assert_eq!(pb.calls(), vec!["noocr pages=2-3".to_string()]);
}

#[test]
fn pdf__nearly_empty_text_retries_with_ocr() {
    let pb = PdfBox::new();
    pb.install_fake_liteparse();
    pb.config(&pb.fake_extractor_path(), &cfg(""));
    pb.set_text("noocr", None, "  ");
    pb.set_text(
        "ocr",
        None,
        "text that only ocr could read, comfortably past twenty characters",
    );
    let f = pb.write_pdf("scan.pdf");
    let out = pdf(&pb, &[f.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let s = stdout(&out);
    assert!(s.contains("ocr: fallback"), "{s}");
    assert_eq!(
        pb.calls(),
        vec!["noocr pages=-".to_string(), "ocr pages=-".to_string()]
    );
    let sink_line = s.lines().find(|l| l.starts_with("sink:")).unwrap();
    assert!(sink_line.contains("-ocr.txt"), "{sink_line}");
}

#[test]
fn pdf__forced_ocr_skips_the_fast_pass() {
    let pb = PdfBox::new();
    pb.install_fake_liteparse();
    pb.config(&pb.fake_extractor_path(), &cfg(""));
    pb.set_text("ocr", None, "ocr-only text");
    let f = pb.write_pdf("scan2.pdf");
    let out = pdf(&pb, &[f.to_str().unwrap(), "--ocr"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("ocr: forced"));
    assert_eq!(pb.calls(), vec!["ocr pages=-".to_string()]);
}

#[test]
fn pdf__extractor_failure_is_refused_and_declared() {
    let pb = PdfBox::new();
    pb.install_fake_liteparse();
    pb.config(&pb.fake_extractor_path(), &cfg(""));
    pb.set_fail("cannot open the document");
    let f = pb.write_pdf("broken.pdf");
    let out = pdf(&pb, &[f.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let e = stderr(&out);
    assert!(e.contains("extractor failed"), "{e}");
    assert!(e.contains("cannot open the document"), "{e}");
    let sink_dir = pb.home.path().join("out/pdf");
    let txt_sinks: Vec<String> = match std::fs::read_dir(&sink_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".txt"))
            .collect(),
        Err(_) => Vec::new(),
    };
    assert!(
        txt_sinks.is_empty(),
        "no sink file should be written on extractor failure, found: {txt_sinks:?}"
    );
}

#[test]
fn pdf__extractor_timeout_is_refused_without_a_stack_trace() {
    let pb = PdfBox::new();
    pb.install_fake_liteparse();
    pb.config(&pb.fake_extractor_path(), &cfg(""));
    pb.set_timeout();
    let f = pb.write_pdf("slow.pdf");
    let out = pdf(&pb, &[f.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let e = stderr(&out);
    assert!(e.contains("extractor failed"), "{e}");
    assert!(e.contains("timeout"), "{e}");
    assert_eq!(e.trim().lines().count(), 1, "no stack trace: {e}");
}

#[test]
fn pdf__header_only_output_never_the_body() {
    let pb = PdfBox::new();
    pb.install_fake_liteparse();
    pb.config(&pb.fake_extractor_path(), &cfg(""));
    let long = "SECRET-BODY-LINE ".repeat(500);
    pb.set_text("noocr", None, &long);
    let f = pb.write_pdf("long.pdf");
    let out = pdf(&pb, &[f.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let s = stdout(&out);
    assert!(
        !s.contains("SECRET-BODY-LINE"),
        "body leaked to stdout: {s}"
    );
    assert!(s.contains("sink:"), "{s}");
    assert!(s.trim().lines().count() <= 6, "header must stay short: {s}");
}
