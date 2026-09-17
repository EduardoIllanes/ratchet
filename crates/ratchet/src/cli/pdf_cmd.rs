//! `ratchet pdf <file> [--pages <range>] [--ocr]`. Exit 0 on success, 1 on any refusal or
//! failure — the ordinary CLI convention, not the hook 0/2 convention (spec D-p3 is about
//! hooks; `ratchet pdf` is never called from one).

use std::collections::HashMap;
use std::path::Path;

pub fn run(file: &Path, pages: Option<&str>, ocr: bool, env: &HashMap<String, String>) -> i32 {
    let home = crate::config::ratchet_home(env);
    let settings = match crate::config::load_machine_config(&home) {
        Ok(mc) => mc.pdf,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    if let Some(p) = pages {
        if let Err(e) = crate::pdf::validate_pages(p) {
            eprintln!("error: {}", e.message());
            return 1;
        }
    }

    if let Err(e) = crate::pdf::check_input_file(file, settings.max_file_bytes) {
        eprintln!("error: {}", e.message());
        return 1;
    }

    let extractor = match crate::pdf::extractor_for(&settings) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {}", e.message());
            return 1;
        }
    };

    let out_dir = home.join("out").join("pdf");
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("error: could not create the sink directory: {e}");
        return 1;
    }
    // Ruling GP-R5 (overridden by GP-P7): a per-process scratch file. The extractor needs an
    // `-o <out>` target before the final sink name is known (it depends on the OCR mode,
    // decided only after extraction). A fixed name let two concurrent `ratchet pdf` runs read
    // and delete each other's scratch file (GP-P7 / B-1); the pid suffix makes collision
    // impossible without changing anything else about the scratch's lifecycle.
    let scratch = out_dir.join(format!(".scratch-{}.txt", std::process::id()));

    let ocr_flag = if ocr { Some(true) } else { None };
    let (run, mode, _first) = match crate::pdf::extract_text(
        extractor.as_ref(),
        file,
        &scratch,
        ocr_flag,
        pages,
        &settings,
    ) {
        Ok(r) => r,
        Err(e) => {
            // The fast pass may already have written the scratch file before the OCR pass (or
            // the fast pass itself) failed. On any failure, no `.txt` sink may survive under
            // `<home>/out/pdf/` (GP-P3), and the scratch file is one.
            let _ = std::fs::remove_file(&scratch);
            eprintln!("error: {}", e.message());
            return 1;
        }
    };
    let _ = std::fs::remove_file(&scratch);

    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "pdf".to_string());
    let mut name = stem;
    if let Some(p) = pages {
        name.push_str("-p");
        name.push_str(p);
    }
    // Ruling GP-R4: the suffix reflects what happened, not what was requested.
    if matches!(mode, "forced" | "fallback") {
        name.push_str("-ocr");
    }
    name.push_str(".txt");
    let sink = out_dir.join(name);
    if let Err(e) = std::fs::write(&sink, &run.text) {
        eprintln!("error: could not write the sink file: {e}");
        return 1;
    }

    // Header format is fixed verbatim — Task 1's scenario tests assert on these exact prefixes.
    println!("pdf: {}", file.display());
    println!("pages: {}", pages.unwrap_or("all"));
    println!("ocr: {mode}");
    println!("chars: {}", run.text.chars().count());
    println!("sink: {}", sink.display());
    0
}
