//! Fetch + index https://models.dev/api.json

use std::collections::HashMap;

use serde::Deserialize;

use super::{CatalogError, Result};

const MODELS_DEV_URL: &str = "https://models.dev/api.json";

#[derive(Debug, Clone, Default)]
pub struct ModelsDevCatalog {
    /// Exact model id → meta (across all providers).
    by_id: HashMap<String, ModelMeta>,
    /// provider_id → (model_id → meta)
    by_provider: HashMap<String, HashMap<String, ModelMeta>>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelMeta {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub attachment: bool,
    pub reasoning: bool,
    pub tool_call: bool,
    pub input_modalities: Vec<String>,
    pub context: u64,
    pub max_output: u64,
    /// USD per 1M input tokens (0 if unknown).
    pub cost_input: f64,
    /// USD per 1M output tokens (0 if unknown).
    pub cost_output: f64,
}

impl ModelMeta {
    pub fn has_image(&self) -> bool {
        self.attachment
            || self
                .input_modalities
                .iter()
                .any(|m| m.eq_ignore_ascii_case("image"))
    }

    pub fn has_video(&self) -> bool {
        self.input_modalities
            .iter()
            .any(|m| m.eq_ignore_ascii_case("video"))
    }

    pub fn has_audio(&self) -> bool {
        self.input_modalities
            .iter()
            .any(|m| m.eq_ignore_ascii_case("audio"))
    }
}

#[derive(Debug, Deserialize)]
struct ProviderEntry {
    id: Option<String>,
    models: Option<HashMap<String, RawModel>>,
}

#[derive(Debug, Deserialize)]
struct RawModel {
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    attachment: bool,
    #[serde(default)]
    reasoning: bool,
    #[serde(default)]
    tool_call: bool,
    modalities: Option<RawModalities>,
    limit: Option<RawLimit>,
    cost: Option<RawCost>,
}

#[derive(Debug, Deserialize)]
struct RawModalities {
    #[serde(default)]
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawLimit {
    #[serde(default)]
    context: u64,
    #[serde(default)]
    output: u64,
}

#[derive(Debug, Deserialize)]
struct RawCost {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
}

pub async fn fetch_models_dev() -> Result<ModelsDevCatalog> {
    let client = reqwest::Client::new();
    let resp = client
        .get(MODELS_DEV_URL)
        .send()
        .await
        .map_err(|e| CatalogError::Http(e.to_string()))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CatalogError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(CatalogError::Api(format!(
            "models.dev {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    parse_models_dev_json(&text)
}

pub(crate) fn parse_models_dev_json(text: &str) -> Result<ModelsDevCatalog> {
    let root: HashMap<String, ProviderEntry> =
        serde_json::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
    let mut catalog = ModelsDevCatalog::default();
    for (provider_key, entry) in root {
        let provider_id = entry.id.unwrap_or(provider_key);
        let Some(models) = entry.models else {
            continue;
        };
        let mut map = HashMap::new();
        for (key, raw) in models {
            let id = raw.id.unwrap_or_else(|| key.clone());
            let meta = ModelMeta {
                id: id.clone(),
                name: raw.name.unwrap_or_else(|| key.clone()),
                provider_id: provider_id.clone(),
                attachment: raw.attachment,
                reasoning: raw.reasoning,
                tool_call: raw.tool_call,
                input_modalities: raw.modalities.map(|m| m.input).unwrap_or_default(),
                context: raw.limit.as_ref().map(|l| l.context).unwrap_or(0),
                max_output: raw.limit.as_ref().map(|l| l.output).unwrap_or(0),
                cost_input: raw.cost.as_ref().map(|c| c.input).unwrap_or(0.0),
                cost_output: raw.cost.as_ref().map(|c| c.output).unwrap_or(0.0),
            };
            catalog.by_id.insert(id.clone(), meta.clone());
            map.insert(id, meta);
        }
        catalog.by_provider.insert(provider_id, map);
    }
    Ok(catalog)
}

impl ModelsDevCatalog {
    /// Look up metadata: exact id, then provider-scoped id, then suffix match.
    pub fn lookup(&self, model_id: &str, provider_hint: Option<&str>) -> Option<&ModelMeta> {
        if let Some(m) = self.by_id.get(model_id) {
            return Some(m);
        }
        if let Some(hint) = provider_hint {
            if let Some(map) = self.by_provider.get(hint) {
                if let Some(m) = map.get(model_id) {
                    return Some(m);
                }
            }
        }
        // Suffix / tail match (e.g. short id vs full path).
        let tail = model_id.rsplit('/').next().unwrap_or(model_id);
        self.by_id.values().find(|m| {
            m.id == tail || m.id.ends_with(&format!("/{tail}")) || m.id.ends_with(model_id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_blob() {
        let json = r#"{
          "fireworks-ai": {
            "id": "fireworks-ai",
            "models": {
              "accounts/fireworks/models/kimi": {
                "id": "accounts/fireworks/models/kimi",
                "name": "Kimi",
                "attachment": true,
                "reasoning": true,
                "tool_call": true,
                "modalities": { "input": ["text", "image"], "output": ["text"] },
                "limit": { "context": 128000, "output": 8192 },
                "cost": { "input": 2.1, "output": 6.6, "cache_read": 0.3, "cache_write": 0 }
              }
            }
          }
        }"#;
        let cat = parse_models_dev_json(json).unwrap();
        let m = cat
            .lookup("accounts/fireworks/models/kimi", Some("fireworks-ai"))
            .unwrap();
        assert!(m.has_image());
        assert!(m.reasoning);
        assert_eq!(m.context, 128000);
        assert!((m.cost_input - 2.1).abs() < 0.001);
        assert!((m.cost_output - 6.6).abs() < 0.001);
    }
}
