//! Two-column label + host rows (provider lists in intro / `/connect`).

/// Host part of an OpenAI-compatible base URL (`api.openai.com`, `127.0.0.1:11434`).
pub(crate) fn host_hint(base_url: &str) -> &str {
    base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
}

pub(crate) fn ellipsize(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Left label + right-aligned host; truncates host (then label) so they never collide.
pub(crate) fn layout_label_host(label: &str, host: &str, width: usize) -> (String, String, usize) {
    if width == 0 {
        return (String::new(), String::new(), 0);
    }
    if host.is_empty() {
        return (ellipsize(label, width), String::new(), 0);
    }

    const MIN_GAP: usize = 1;
    let label_w = label.chars().count();
    let host_w = host.chars().count();
    if label_w + MIN_GAP + host_w <= width {
        return (
            label.to_string(),
            host.to_string(),
            width - label_w - host_w,
        );
    }

    // Prefer a readable label; squeeze the host first.
    let label_max = width.saturating_sub(MIN_GAP + 4).max(1);
    let left = ellipsize(label, label_max.min(label_w));
    let left_w = left.chars().count();
    let right = ellipsize(host, width.saturating_sub(left_w + MIN_GAP));
    let right_w = right.chars().count();
    let gap = width.saturating_sub(left_w + right_w);
    (left, right, gap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_host_does_not_collide_with_label() {
        let (left, right, gap) =
            layout_label_host("Google AI", "generativelanguage.googleapis.com", 40);
        assert_eq!(left.chars().count() + gap + right.chars().count(), 40);
        assert!(gap >= 1);
        assert!(right.ends_with('…'), "{right}");
        assert!(!left.contains("generative"), "{left}");
    }

    #[test]
    fn host_hint_strips_scheme_and_path() {
        assert_eq!(host_hint("https://api.openai.com/v1"), "api.openai.com");
        assert_eq!(host_hint("http://127.0.0.1:11434/v1"), "127.0.0.1:11434");
        assert_eq!(host_hint(""), "");
    }
}
