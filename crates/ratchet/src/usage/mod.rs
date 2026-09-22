//! Token cost per task, role and model, read from the transcripts Claude Code already writes to
//! disk. Every module here except the CLI face (`crate::cli::usage_cmd`, Task 4) is pure: no
//! filesystem, no clock, no database. See
//! `docs/superpowers/specs/2026-09-21-ratchet-usage-design.md`.

pub mod attribute;
pub mod transcript;
pub mod weights;
