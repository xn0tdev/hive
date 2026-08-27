//! `GET {base}/models` — OpenAI-compatible listing.

use serde::Deserialize;
use serde_json::Value;

use super::{CatalogError, Result};

/// One id returned by the provider (before models.dev enrichment).
#[derive(Debug, Clone, Default)]
pub struct RemoteModel {
    pub id: String,
    pub name: Option<String>,
    /// Priced at zero by the provider's own listing. `None` when it quoted no
    /// price at all — models.dev gets to answer in that case.
    pub free: Option<bool>,
}

/// Versioned Fireworks ids that `GET /models` sometimes omits (esp. `/routers`).
/// Kept in sync with hive defaults + Fireworks serverless Fast paths.
const FIREWORKS_SUPPLEMENT: &[(&str, &str)] = &[
    ("accounts/fireworks/models/glm-5p2", "GLM 5.2"),
    ("accounts/fireworks/routers/glm-5p2-fast", "GLM 5.2 Fast"),
    ("accounts/fireworks/models/kimi-k2p6", "Kimi K2.6"),
    (
        "accounts/fireworks/routers/kimi-k2p6-fast",
        "Kimi K2.6 Fast",
    ),
    ("accounts/fireworks/models/kimi-k3", "Kimi K3"),
];

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

/// List models from an OpenAI-compatible provider endpoint.
pub async fn list_provider_models(base_url: &str, api_key: &str) -> Result<Vec<RemoteModel>> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/models");
    let resp = crate::auth::with_auth(super::http::client().get(&url), api_key, base_url)
        .send()
        .await
        .map_err(|e| CatalogError::Http(e.to_string()))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CatalogError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(CatalogError::Api(format!("{status}: {text}")));
    }
    let mut models = parse_models_json(&text)?;
    merge_fireworks_supplements(base_url, &mut models);
    Ok(models)
}

/// Parse OpenAI-style `{ "data": [...] }`, bare `[...]`, or `{ "models": [...] }`.
pub(crate) fn parse_models_json(text: &str) -> Result<Vec<RemoteModel>> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
    let entries = match value {
        Value::Array(arr) => arr,
        Value::Object(map) => map
            .get("data")
            .or_else(|| map.get("models"))
            .cloned()
            .and_then(|v| match v {
                Value::Array(a) => Some(a),
                _ => None,
            })
            .unwrap_or_default(),
        _ => {
            return Err(CatalogError::Parse(
                "expected object with data/models array or a bare array".into(),
            ));
        }
    };

    let mut out = Vec::new();
    for entry in entries {
        let Some(model) = remote_from_value(entry) else {
            continue;
        };
        out.push(model);
    }
    sort_dedup_models(&mut out);
    Ok(out)
}

/// A price that may arrive as a number or, as OpenRouter sends it, a string.
fn price_of(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Read `pricing` off a listing entry: zero on both sides means free.
///
/// OpenRouter's `:free` id suffix looks tempting but under-counts — it misses
/// free models that don't carry it — so the quoted price is what decides.
fn free_from_pricing(entry: &Value) -> Option<bool> {
    let pricing = entry.get("pricing")?;
    let input = price_of(pricing.get("prompt").or_else(|| pricing.get("input")))?;
    let output = price_of(pricing.get("completion").or_else(|| pricing.get("output")))?;
    Some(input == 0.0 && output == 0.0)
}

fn remote_from_value(entry: Value) -> Option<RemoteModel> {
    let free = free_from_pricing(&entry);
    // Prefer typed decode when it looks like an object with `id`.
    if let Ok(m) = serde_json::from_value::<ModelEntry>(entry.clone()) {
        let id = m.id.trim().to_string();
        if id.is_empty() {
            return None;
        }
        let name = m
            .name
            .or(m.display_name)
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty());
        return Some(RemoteModel { id, name, free });
    }
    // Bare string id.
    if let Some(id) = entry.as_str() {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        return Some(RemoteModel {
            id: id.to_string(),
            ..Default::default()
        });
    }
    // Loose object: id + optional name-like fields.
    let obj = entry.as_object()?;
    let id = obj
        .get("id")
        .or_else(|| obj.get("model"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let name = ["name", "display_name", "displayName", "title"]
        .iter()
        .find_map(|k| {
            obj.get(*k)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
    Some(RemoteModel {
        id: id.to_string(),
        name,
        free,
    })
}

fn is_fireworks_base(base_url: &str) -> bool {
    base_url.to_ascii_lowercase().contains("api.fireworks.ai")
}

/// Inject known Fireworks models when the listing omits versioned / router ids.
fn merge_fireworks_supplements(base_url: &str, models: &mut Vec<RemoteModel>) {
    if !is_fireworks_base(base_url) {
        return;
    }
    for (id, name) in FIREWORKS_SUPPLEMENT {
        if models.iter().any(|m| m.id == *id) {
            continue;
        }
        models.push(RemoteModel {
            id: (*id).into(),
            name: Some((*name).into()),
            free: None,
        });
    }
    sort_dedup_models(models);
}

fn sort_dedup_models(models: &mut Vec<RemoteModel>) {
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_style_list() {
        let json = r#"{"object":"list","data":[
            {"id":"gpt-4o","object":"model"},
            {"id":"accounts/fireworks/models/kimi","name":"Kimi"}
        ]}"#;
        let models = parse_models_json(json).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "accounts/fireworks/models/kimi");
        assert_eq!(models[0].name.as_deref(), Some("Kimi"));
        assert_eq!(models[1].id, "gpt-4o");
    }

    /// OpenRouter quotes prices as strings, and zero on both sides means free.
    #[test]
    fn reads_free_from_openrouter_pricing() {
        let json = r#"{"data":[
            {"id":"nvidia/nemotron:free","pricing":{"prompt":"0","completion":"0"}},
            {"id":"anthropic/claude","pricing":{"prompt":"0.000003","completion":"0.000015"}},
            {"id":"unknown/model"}
        ]}"#;
        let models = parse_models_json(json).unwrap();
        let by_id = |id: &str| models.iter().find(|m| m.id == id).unwrap().free;
        assert_eq!(by_id("nvidia/nemotron:free"), Some(true));
        assert_eq!(by_id("anthropic/claude"), Some(false));
        assert_eq!(
            by_id("unknown/model"),
            None,
            "no price quoted is not 'paid'"
        );
    }

    /// A free model without the `:free` suffix must still be caught — the
    /// suffix under-counts, the price does not.
    #[test]
    fn free_does_not_depend_on_the_id_suffix() {
        let json = r#"{"data":[{"id":"vendor/plain","pricing":{"prompt":0,"completion":0}}]}"#;
        let models = parse_models_json(json).unwrap();
        assert_eq!(models[0].free, Some(true));
    }

    /// Providers that name the fields `input`/`output` work the same.
    #[test]
    fn reads_free_from_input_output_pricing() {
        let json = r#"{"data":[{"id":"a","pricing":{"input":0,"output":0}}]}"#;
        let models = parse_models_json(json).unwrap();
        assert_eq!(models[0].free, Some(true));
    }

    /// Half-priced is not free.
    #[test]
    fn output_priced_is_not_free() {
        let json = r#"{"data":[{"id":"a","pricing":{"prompt":"0","completion":"0.5"}}]}"#;
        let models = parse_models_json(json).unwrap();
        assert_eq!(models[0].free, Some(false));
    }

    #[test]
    fn parses_bare_array() {
        let json = r#"[{"id":"a"},{"id":"b","display_name":"Bee"}]"#;
        let models = parse_models_json(json).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "a");
        assert_eq!(models[1].name.as_deref(), Some("Bee"));
    }

    #[test]
    fn parses_models_key_and_string_ids() {
        let json = r#"{"models":["org/foo","org/bar"]}"#;
        let models = parse_models_json(json).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "org/bar");
        assert_eq!(models[1].id, "org/foo");
    }

    #[test]
    fn skips_empty_ids() {
        let json = r#"{"data":[{"id":""},{"id":"  "},{"id":"ok"}]}"#;
        let models = parse_models_json(json).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "ok");
    }

    #[test]
    fn fireworks_supplement_adds_missing_versioned_ids() {
        let mut models = vec![RemoteModel {
            id: "accounts/fireworks/models/kimi".into(),
            name: Some("Kimi".into()),
            free: None,
        }];
        merge_fireworks_supplements("https://api.fireworks.ai/inference/v1", &mut models);
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"accounts/fireworks/models/glm-5p2"));
        assert!(ids.contains(&"accounts/fireworks/routers/glm-5p2-fast"));
        assert!(ids.contains(&"accounts/fireworks/models/kimi-k2p6"));
        assert!(ids.contains(&"accounts/fireworks/routers/kimi-k2p6-fast"));
        assert!(ids.contains(&"accounts/fireworks/models/kimi"));
    }

    #[test]
    fn fireworks_supplement_skips_non_fireworks() {
        let mut models = vec![RemoteModel {
            id: "gpt-4o".into(),
            ..Default::default()
        }];
        merge_fireworks_supplements("https://api.openai.com/v1", &mut models);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-4o");
    }

    #[test]
    fn fireworks_supplement_does_not_duplicate() {
        let mut models = vec![RemoteModel {
            id: "accounts/fireworks/models/glm-5p2".into(),
            name: Some("from-api".into()),
            free: None,
        }];
        merge_fireworks_supplements("https://api.fireworks.ai/inference/v1", &mut models);
        let glm: Vec<_> = models
            .iter()
            .filter(|m| m.id == "accounts/fireworks/models/glm-5p2")
            .collect();
        assert_eq!(glm.len(), 1);
        assert_eq!(glm[0].name.as_deref(), Some("from-api"));
    }
}
