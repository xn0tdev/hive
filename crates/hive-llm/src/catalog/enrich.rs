use super::models_dev::ModelsDevCatalog;
use super::provider_list::RemoteModel;

/// Remote model + optional models.dev facts for the UI.
#[derive(Debug, Clone)]
pub struct ModelCard {
    pub id: String,
    pub name: String,
    pub vision: bool,
    pub video: bool,
    pub audio: bool,
    pub reasoning: bool,
    pub tools: bool,
    pub context: u64,
    /// True when models.dev had a hit (otherwise capability flags are unknown/false).
    pub enriched: bool,
}

impl ModelCard {
    /// Compact badge line for the picker, e.g. `128k · vision · tools`.
    pub fn badges(&self) -> String {
        let mut parts = Vec::new();
        if self.context > 0 {
            parts.push(format_context(self.context));
        }
        if self.vision {
            parts.push("vision".into());
        }
        if self.video {
            parts.push("video".into());
        }
        if self.audio {
            parts.push("audio".into());
        }
        if self.reasoning {
            parts.push("reason".into());
        }
        if self.tools {
            parts.push("tools".into());
        }
        if !self.enriched && parts.is_empty() {
            parts.push("listed".into());
        }
        parts.join(" · ")
    }
}

fn format_context(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{}m", n / 1_000_000)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

pub fn enrich_models(
    remote: &[RemoteModel],
    catalog: Option<&ModelsDevCatalog>,
    provider_hint: Option<&str>,
) -> Vec<ModelCard> {
    remote
        .iter()
        .map(|r| {
            let meta = catalog.and_then(|c| c.lookup(&r.id, provider_hint));
            let name = r
                .name
                .clone()
                .or_else(|| meta.map(|m| m.name.clone()))
                .unwrap_or_else(|| short_id(&r.id));
            match meta {
                Some(m) => ModelCard {
                    id: r.id.clone(),
                    name,
                    vision: m.has_image(),
                    video: m.has_video(),
                    audio: m.has_audio(),
                    reasoning: m.reasoning,
                    tools: m.tool_call,
                    context: m.context,
                    enriched: true,
                },
                None => ModelCard {
                    id: r.id.clone(),
                    name,
                    vision: false,
                    video: false,
                    audio: false,
                    reasoning: false,
                    tools: false,
                    context: 0,
                    enriched: false,
                },
            }
        })
        .collect()
}

fn short_id(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

#[cfg(test)]
mod tests {
    use super::super::models_dev::parse_models_dev_json;
    use super::*;

    #[test]
    fn enriches_vision_from_catalog() {
        let cat = parse_models_dev_json(
            r#"{
          "fireworks-ai": {
            "id": "fireworks-ai",
            "models": {
              "foo": {
                "id": "foo",
                "name": "Foo",
                "attachment": true,
                "tool_call": true,
                "modalities": { "input": ["text", "image"] },
                "limit": { "context": 200000 }
              }
            }
          }
        }"#,
        )
        .unwrap();
        let cards = enrich_models(
            &[RemoteModel {
                id: "foo".into(),
                name: None,
            }],
            Some(&cat),
            Some("fireworks-ai"),
        );
        assert!(cards[0].vision);
        assert!(cards[0].badges().contains("vision"));
        assert!(cards[0].badges().contains("200k"));
    }
}
