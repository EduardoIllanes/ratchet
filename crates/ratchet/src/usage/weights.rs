//! Cost is optional and comes only from configured weights (D-usage-weights): pure matching and
//! arithmetic, no TOML parsing here — `config.rs` already owns deserializing
//! `[usage.weights."<prefix>"]` into `HashMap<String, ModelWeights>`. Takes plain token counts
//! rather than a `usage::attribute::Totals`, so this module has no dependency on
//! `usage::attribute` (Task 3) and stays buildable on its own in this task.

use std::collections::HashMap;

use crate::config::ModelWeights;

/// The configured weight whose prefix is the longest match of `model`, or `None`.
// Consumed by usage::attribute (Task 3).
#[allow(dead_code)]
pub fn matching<'a>(
    model: &str,
    weights: &'a HashMap<String, ModelWeights>,
) -> Option<&'a ModelWeights> {
    weights
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, w)| w)
}

/// `None` when no weight matches `model` — callers must never print a cost in that case
/// (D-usage-weights: "without a matching weight, tokens only, never invent a price").
// Consumed by usage::attribute (Task 3).
#[allow(dead_code)]
pub fn cost(
    model: &str,
    weights: &HashMap<String, ModelWeights>,
    input: u64,
    cache_write: u64,
    cache_read: u64,
    output: u64,
) -> Option<f64> {
    let w = matching(model, weights)?;
    let per = |n: u64, rate: f64| (n as f64) * rate / 1_000_000.0;
    Some(
        per(input, w.input)
            + per(cache_write, w.cache_write)
            + per(cache_read, w.cache_read)
            + per(output, w.output),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights(pairs: &[(&str, f64, f64, f64, f64)]) -> HashMap<String, ModelWeights> {
        pairs
            .iter()
            .map(|(prefix, input, cache_write, cache_read, output)| {
                (
                    prefix.to_string(),
                    ModelWeights {
                        input: *input,
                        cache_write: *cache_write,
                        cache_read: *cache_read,
                        output: *output,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn no_prefix_matches_gives_none() {
        let w = weights(&[("claude-sonnet", 1.0, 1.0, 1.0, 1.0)]);
        assert!(matching("claude-opus-5", &w).is_none());
        assert!(cost("claude-opus-5", &w, 1000, 0, 0, 0).is_none());
    }

    #[test]
    fn the_longest_matching_prefix_wins() {
        let w = weights(&[
            ("claude", 1.0, 0.0, 0.0, 0.0),
            ("claude-opus", 100.0, 0.0, 0.0, 0.0),
        ]);
        let matched = matching("claude-opus-5", &w).unwrap();
        assert_eq!(matched.input, 100.0);
    }

    #[test]
    fn cost_sums_all_four_classes_per_million() {
        let w = weights(&[("claude-sonnet", 3.0, 3.75, 0.3, 15.0)]);
        let c = cost(
            "claude-sonnet-5",
            &w,
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
        )
        .unwrap();
        assert!((c - (3.0 + 3.75 + 0.3 + 15.0)).abs() < 1e-9, "{c}");
    }

    #[test]
    fn zero_tokens_cost_zero_even_with_weights() {
        let w = weights(&[("claude-sonnet", 3.0, 3.75, 0.3, 15.0)]);
        assert_eq!(cost("claude-sonnet-5", &w, 0, 0, 0, 0), Some(0.0));
    }
}
