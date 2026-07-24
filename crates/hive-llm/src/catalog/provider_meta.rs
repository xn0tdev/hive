//! Map OpenAI-compatible base URLs to a UI label + models.dev hint.

/// Known OpenAI-compatible hosts → (label, models.dev provider id).
const KNOWN: &[(&str, &str, Option<&str>)] = &[
    ("api.fireworks.ai", "Fireworks", Some("fireworks-ai")),
    ("openrouter.ai", "OpenRouter", Some("openrouter")),
    ("api.openai.com", "OpenAI", Some("openai")),
    ("api.groq.com", "Groq", Some("groq")),
    ("api.deepseek.com", "DeepSeek", Some("deepseek")),
    ("api.together.xyz", "Together", Some("togetherai")),
    ("api.mistral.ai", "Mistral", Some("mistral")),
    ("generativelanguage.googleapis.com", "Google AI", Some("google")),
    ("api.x.ai", "xAI", Some("xai")),
    ("api.cerebras.ai", "Cerebras", Some("cerebras")),
    ("api.sambanova.ai", "SambaNova", Some("sambanova")),
    ("api.hyperbolic.xyz", "Hyperbolic", Some("hyperbolic")),
    ("127.0.0.1:11434", "Ollama", Some("ollama")),
    ("localhost:11434", "Ollama", Some("ollama")),
    ("127.0.0.1:1234", "LM Studio", None),
    ("localhost:1234", "LM Studio", None),
];

/// Pretty provider name for the model picker section header.
pub fn provider_label_for_base(base_url: &str) -> &'static str {
    let lower = base_url.to_ascii_lowercase();
    for (host, label, _) in KNOWN {
        if lower.contains(host) {
            return label;
        }
    }
    "Provider"
}

/// models.dev catalog hint for enrichment, when known.
pub fn models_dev_hint_for_base(base_url: &str) -> Option<&'static str> {
    let lower = base_url.to_ascii_lowercase();
    for (host, _, hint) in KNOWN {
        if lower.contains(host) {
            return *hint;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fireworks_label_and_hint() {
        assert_eq!(
            provider_label_for_base("https://api.fireworks.ai/inference/v1"),
            "Fireworks"
        );
        assert_eq!(
            models_dev_hint_for_base("https://api.fireworks.ai/inference/v1"),
            Some("fireworks-ai")
        );
    }

    #[test]
    fn openrouter_is_one_provider_label() {
        assert_eq!(
            provider_label_for_base("https://openrouter.ai/api/v1"),
            "OpenRouter"
        );
    }
}
