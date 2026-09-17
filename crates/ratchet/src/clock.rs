//! One clock for the whole binary. Services never read it: they take `now` as a parameter, so a
//! test can place an event ninety minutes in the past without sleeping. The faces read it here,
//! and `RATCHET_NOW` (RFC 3339) replaces it — a documented test seam, harmless in a local tool.

use std::collections::HashMap;

use chrono::{DateTime, SecondsFormat, Utc};

// Consumed by the faces once wired: hooks::dispatch (Task 10) and cli::session_cmd (Task 11).
#[allow(dead_code)]
pub fn now(env: &HashMap<String, String>) -> DateTime<Utc> {
    env.get("RATCHET_NOW")
        .and_then(|s| parse(s))
        .unwrap_or_else(Utc::now)
}

/// The single stored/printed form: UTC, seconds precision, `Z`. Sorts lexicographically.
// Consumed by services::events::emit (Task 6), services::sessions (Task 7) and the faces
// (Tasks 10, 11) once they format timestamps for storage or display.
pub fn iso(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn parse(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(value: &str) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("RATCHET_NOW".to_string(), value.to_string());
        env
    }

    #[test]
    fn now_honours_the_seam() {
        let ts = now(&env_with("2026-09-16T12:00:00Z"));
        assert_eq!(iso(ts), "2026-09-16T12:00:00Z");
    }

    #[test]
    fn now_falls_back_to_the_clock_when_the_seam_is_absent_or_junk() {
        assert!(now(&HashMap::new()).timestamp() > 0);
        assert!(now(&env_with("not a date")).timestamp() > 0);
    }

    #[test]
    fn iso_is_utc_seconds_with_a_z() {
        let ts = parse("2026-09-16T09:00:00-03:00").unwrap();
        assert_eq!(iso(ts), "2026-09-16T12:00:00Z");
    }

    #[test]
    fn iso_strings_sort_like_instants() {
        let mut v = [
            iso(parse("2026-09-16T12:00:00Z").unwrap()),
            iso(parse("2026-01-02T03:04:05Z").unwrap()),
        ];
        v.sort();
        assert_eq!(v[0], "2026-01-02T03:04:05Z");
    }
}
