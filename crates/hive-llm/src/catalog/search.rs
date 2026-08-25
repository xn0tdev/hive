//! Model picker / intro filter: "Grok 4.6" should hit `grok-4.6`, `grok-4p6`,
//! and `x-ai/grok-4.6`, not only the one listing that spelled the name with a
//! space.

/// True when `query` matches any of the haystack fields (id, name, group, …).
///
/// Empty query matches everything. Punctuation, spaces, and Fireworks-style
/// `4p6` decimals are ignored so the same model from different providers
/// survives the same search.
pub fn query_matches(query: &str, fields: &[&str]) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let ql = q.to_ascii_lowercase();
    if fields.iter().any(|f| f.to_ascii_lowercase().contains(&ql)) {
        return true;
    }
    let hay = normalize(&fields.join(" "));
    q.split_whitespace()
        .map(normalize)
        .filter(|t| !t.is_empty())
        .all(|t| hay.contains(&t))
}

/// Strip separators. `p` between digits (`kimi-k2p6`) is a decimal point.
fn normalize(s: &str) -> String {
    let chars: Vec<char> = s.to_ascii_lowercase().chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == 'p' {
            let digit_either_side = i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
            if digit_either_side {
                continue;
            }
        }
        if c.is_ascii_alphanumeric() {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_anything() {
        assert!(query_matches("", &["x"]));
        assert!(query_matches("   ", &["x"]));
    }

    #[test]
    fn spaced_name_hits_hyphen_and_dot_ids() {
        assert!(query_matches(
            "Grok 4.6",
            &["x-ai/grok-4.6", "Grok 4.6", "OpenRouter"]
        ));
        assert!(query_matches("Grok 4.6", &["grok-4.6", "grok-4.6", "xAI"]));
        assert!(query_matches(
            "Grok 4.6",
            &[
                "accounts/fireworks/models/grok-4p6",
                "grok-4p6",
                "Fireworks"
            ]
        ));
        assert!(query_matches(
            "Grok 4.6",
            &["grok-4-6", "Grok-4-6", "Venice"]
        ));
    }

    #[test]
    fn hyphenated_query_hits_pretty_name() {
        assert!(query_matches(
            "grok-4.6",
            &["x-ai/grok-4.6", "Grok 4.6", "OpenRouter"]
        ));
        assert!(query_matches(
            "kimi-k2.6",
            &["accounts/fireworks/models/kimi-k2p6", "Kimi K2.6"]
        ));
    }

    #[test]
    fn tokens_must_all_match() {
        assert!(query_matches("grok xai", &["grok-4.6", "Grok 4.6", "xAI"]));
        assert!(!query_matches(
            "grok fireworks",
            &["x-ai/grok-4.6", "Grok 4.6", "OpenRouter"]
        ));
    }

    #[test]
    fn unrelated_models_stay_out() {
        assert!(!query_matches(
            "Grok 4.6",
            &["gpt-4.1", "GPT-4.1", "OpenAI"]
        ));
        assert!(!query_matches("claude", &["grok-4.6", "Grok 4.6", "xAI"]));
    }
}
