//! OpenAI-compatible provider presets for the intro wizard.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    pub label: &'static str,
    /// Short copy for the detail panel under the provider list (2–3 lines when wrapped).
    pub description: &'static str,
    pub base_url: &'static str,
    pub api_key_env: &'static str,
    /// models.dev provider id hint (optional).
    pub models_dev_hint: Option<&'static str>,
    pub is_custom: bool,
}

pub const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        label: "Fireworks",
        description: "Open-source models — Hive's recommended default. Simple OpenAI-compatible API.",
        base_url: "https://api.fireworks.ai/inference/v1",
        api_key_env: "FIREWORKS_API_KEY",
        models_dev_hint: Some("fireworks-ai"),
        is_custom: false,
    },
    ProviderPreset {
        label: "OpenRouter",
        description: "One key for many providers — OpenAI, Anthropic/Claude, open models, and more.",
        base_url: "https://openrouter.ai/api/v1",
        api_key_env: "OPENROUTER_API_KEY",
        models_dev_hint: Some("openrouter"),
        is_custom: false,
    },
    ProviderPreset {
        label: "OpenAI",
        description: "Official GPT models via api.openai.com. Use when you already have an OpenAI key.",
        base_url: "https://api.openai.com/v1",
        api_key_env: "OPENAI_API_KEY",
        models_dev_hint: Some("openai"),
        is_custom: false,
    },
    ProviderPreset {
        label: "Anthropic",
        description: "Claude via Anthropic's OpenAI-compatible API. Paste an Anthropic key.",
        base_url: "https://api.anthropic.com/v1",
        api_key_env: "ANTHROPIC_API_KEY",
        models_dev_hint: Some("anthropic"),
        is_custom: false,
    },
    ProviderPreset {
        label: "Groq",
        description: "Low-latency inference for Llama, GPT-OSS, and more. Best when snappy tool loops matter.",
        base_url: "https://api.groq.com/openai/v1",
        api_key_env: "GROQ_API_KEY",
        models_dev_hint: Some("groq"),
        is_custom: false,
    },
    ProviderPreset {
        label: "DeepSeek",
        description: "DeepSeek's own API — strong coding and reasoning at competitive cost.",
        base_url: "https://api.deepseek.com/v1",
        api_key_env: "DEEPSEEK_API_KEY",
        models_dev_hint: Some("deepseek"),
        is_custom: false,
    },
    ProviderPreset {
        label: "Together",
        description: "Hosted open models with an OpenAI-compatible API — wide open-weight catalog.",
        base_url: "https://api.together.xyz/v1",
        api_key_env: "TOGETHER_API_KEY",
        models_dev_hint: Some("togetherai"),
        is_custom: false,
    },
    ProviderPreset {
        label: "Mistral",
        description: "Mistral Large and open models via api.mistral.ai — native OpenAI-compatible chat.",
        base_url: "https://api.mistral.ai/v1",
        api_key_env: "MISTRAL_API_KEY",
        models_dev_hint: Some("mistral"),
        is_custom: false,
    },
    ProviderPreset {
        label: "Google AI",
        description: "Gemini via Google's OpenAI-compatible endpoint. Paste a Google AI Studio key.",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        api_key_env: "GOOGLE_API_KEY",
        models_dev_hint: Some("google"),
        is_custom: false,
    },
    ProviderPreset {
        label: "xAI",
        description: "Grok models via xAI's OpenAI-compatible API at api.x.ai.",
        base_url: "https://api.x.ai/v1",
        api_key_env: "XAI_API_KEY",
        models_dev_hint: Some("xai"),
        is_custom: false,
    },
    ProviderPreset {
        label: "Cerebras",
        description: "Very fast inference on Cerebras wafers — OpenAI-compatible /v1 endpoint.",
        base_url: "https://api.cerebras.ai/v1",
        api_key_env: "CEREBRAS_API_KEY",
        models_dev_hint: Some("cerebras"),
        is_custom: false,
    },
    ProviderPreset {
        label: "SambaNova",
        description: "SambaCloud inference with an OpenAI-compatible base URL — strong for large open models.",
        base_url: "https://api.sambanova.ai/v1",
        api_key_env: "SAMBANOVA_API_KEY",
        models_dev_hint: Some("sambanova"),
        is_custom: false,
    },
    ProviderPreset {
        label: "Hyperbolic",
        description: "Affordable GPU inference gateway — OpenAI-compatible chat at api.hyperbolic.xyz.",
        base_url: "https://api.hyperbolic.xyz/v1",
        api_key_env: "HYPERBOLIC_API_KEY",
        models_dev_hint: Some("hyperbolic"),
        is_custom: false,
    },
    ProviderPreset {
        label: "Ollama",
        description: "Local models on your machine. Any non-empty string works as the API key.",
        base_url: "http://127.0.0.1:11434/v1",
        api_key_env: "OLLAMA_API_KEY",
        models_dev_hint: Some("ollama"),
        is_custom: false,
    },
    ProviderPreset {
        label: "LM Studio",
        description: "Local OpenAI-compatible server (default port 1234). Any non-empty string works as key.",
        base_url: "http://127.0.0.1:1234/v1",
        api_key_env: "LM_STUDIO_API_KEY",
        models_dev_hint: None,
        is_custom: false,
    },
    ProviderPreset {
        label: "Custom",
        description: "Any OpenAI-compatible endpoint — set base URL, env var name, then paste the key.",
        base_url: "",
        api_key_env: "LLM_API_KEY",
        models_dev_hint: None,
        is_custom: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_env_and_label() {
        for p in PRESETS {
            assert!(!p.label.is_empty());
            assert!(!p.description.is_empty());
            assert!(!p.api_key_env.is_empty());
            if !p.is_custom {
                assert!(!p.base_url.is_empty(), "{}", p.label);
            }
        }
    }

    #[test]
    fn custom_is_last() {
        assert!(PRESETS.last().is_some_and(|p| p.is_custom));
    }

    #[test]
    fn anthropic_preset() {
        let p = PRESETS
            .iter()
            .find(|p| p.label == "Anthropic")
            .expect("Anthropic preset");
        assert_eq!(p.base_url, "https://api.anthropic.com/v1");
        assert_eq!(p.api_key_env, "ANTHROPIC_API_KEY");
        assert_eq!(p.models_dev_hint, Some("anthropic"));
        assert!(!p.is_custom);
        let openai = PRESETS.iter().position(|p| p.label == "OpenAI").unwrap();
        let anthropic = PRESETS.iter().position(|p| p.label == "Anthropic").unwrap();
        assert_eq!(anthropic, openai + 1, "Anthropic sits next to OpenAI");
    }
}
