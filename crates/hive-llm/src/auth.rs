//! Auth headers for OpenAI-compatible hosts.

use reqwest::RequestBuilder;

const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Anthropic's Chat Completions layer still expects an output cap. Other
/// hosts treat a missing `max_tokens` as "model default".
const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 16_384;

pub fn is_anthropic(base_url: &str) -> bool {
    base_url.to_ascii_lowercase().contains("api.anthropic.com")
}

/// Bearer for every host. Anthropic's native `/models` (and some chat paths)
/// also want `x-api-key` + `anthropic-version`.
pub fn with_auth(req: RequestBuilder, api_key: &str, base_url: &str) -> RequestBuilder {
    let req = req.bearer_auth(api_key);
    if is_anthropic(base_url) {
        req.header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
    } else {
        req
    }
}

pub fn default_max_tokens(base_url: &str, requested: Option<u32>) -> Option<u32> {
    if requested.is_some() {
        return requested;
    }
    if is_anthropic(base_url) {
        Some(ANTHROPIC_DEFAULT_MAX_TOKENS)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_anthropic_host() {
        assert!(is_anthropic("https://api.anthropic.com/v1"));
        assert!(is_anthropic("https://API.ANTHROPIC.COM/v1/"));
        assert!(!is_anthropic("https://api.openai.com/v1"));
        assert!(!is_anthropic("https://openrouter.ai/api/v1"));
    }

    #[test]
    fn anthropic_fills_in_max_tokens() {
        assert_eq!(
            default_max_tokens("https://api.anthropic.com/v1", None),
            Some(16_384)
        );
        assert_eq!(
            default_max_tokens("https://api.anthropic.com/v1", Some(512)),
            Some(512)
        );
        assert_eq!(default_max_tokens("https://api.openai.com/v1", None), None);
    }
}
