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
    /// USD per 1M input tokens (0 if unknown).
    pub cost_input: f64,
    /// USD per 1M output tokens (0 if unknown).
    pub cost_output: f64,
    /// Costs nothing to call.
    pub free: bool,
}

impl ModelCard {
    /// Compact badge line for the picker: `128k`, or `128k · FREE`.
    ///
    /// Context and price are the only two things worth scanning a list for.
    /// The capability flags stay on the struct — `suggest` still picks by them
    /// — they just don't earn a column in the picker.
    pub fn badges(&self) -> String {
        let mut parts = Vec::new();
        if self.context > 0 {
            parts.push(format_context(self.context));
        }
        if self.free {
            parts.push("FREE".into());
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

/// Fill in models.dev entries this provider lists in the catalog but omitted
/// from `GET /models` (new ids, `/routers` paths, lagged first-party APIs).
pub fn merge_catalog_models(
    mut remote: Vec<RemoteModel>,
    catalog: Option<&ModelsDevCatalog>,
    provider_hint: Option<&str>,
) -> Vec<RemoteModel> {
    let Some(hint) = provider_hint else {
        return remote;
    };
    let Some(catalog) = catalog else {
        return remote;
    };
    let Some(map) = catalog.provider_models(hint) else {
        return remote;
    };
    for meta in map.values() {
        if remote.iter().any(|m| ids_overlap(&m.id, &meta.id)) {
            continue;
        }
        remote.push(RemoteModel {
            id: meta.id.clone(),
            name: Some(meta.name.clone()),
            free: None,
        });
    }
    remote
}

fn ids_overlap(a: &str, b: &str) -> bool {
    a == b || a.ends_with(&format!("/{b}")) || b.ends_with(&format!("/{a}"))
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
                    cost_input: m.cost_input,
                    cost_output: m.cost_output,
                    // What the provider quotes today beats what the catalog
                    // recorded whenever it was last built.
                    free: r.free.unwrap_or(m.free),
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
                    cost_input: 0.0,
                    cost_output: 0.0,
                    free: r.free.unwrap_or(false),
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
                ..Default::default()
            }],
            Some(&cat),
            Some("fireworks-ai"),
        );
        // Capabilities are still enriched — they just no longer show as badges.
        assert!(cards[0].vision);
        assert!(cards[0].tools);
        assert_eq!(cards[0].badges(), "200k");
    }

    /// The picker column stays down to context and price, whatever the model
    /// can do.
    #[test]
    fn badges_are_only_context_and_price() {
        let mut c = card(None, r#","cost":{"input":2.1,"output":6.6}"#);
        c.vision = true;
        c.video = true;
        c.audio = true;
        c.reasoning = true;
        c.tools = true;
        assert_eq!(c.badges(), "128k");

        c.free = true;
        assert_eq!(c.badges(), "128k · FREE");
    }

    /// Nothing known about it — show nothing rather than a filler word.
    #[test]
    fn an_unknown_model_gets_an_empty_badge_line() {
        let cards = enrich_models(
            &[RemoteModel {
                id: "brand/new".into(),
                ..Default::default()
            }],
            None,
            None,
        );
        assert_eq!(cards[0].badges(), "");
    }

    fn catalog_with_cost(cost: &str) -> super::super::models_dev::ModelsDevCatalog {
        parse_models_dev_json(&format!(
            r#"{{"p":{{"id":"p","models":{{"m":{{"id":"m","name":"M",
               "limit":{{"context":128000}}{cost}}}}}}}}}"#
        ))
        .unwrap()
    }

    fn card(free: Option<bool>, cost: &str) -> ModelCard {
        let cat = catalog_with_cost(cost);
        enrich_models(
            &[RemoteModel {
                id: "m".into(),
                name: None,
                free,
            }],
            Some(&cat),
            Some("p"),
        )
        .remove(0)
    }

    #[test]
    fn a_zero_priced_model_is_badged_free() {
        let c = card(None, r#","cost":{"input":0,"output":0}"#);
        assert!(c.free);
        assert!(c.badges().ends_with("FREE"), "{}", c.badges());
    }

    /// No `cost` block means the price is unknown — that is not free.
    #[test]
    fn a_model_without_a_price_is_not_free() {
        let c = card(None, "");
        assert!(!c.free);
        assert!(!c.badges().contains("FREE"), "{}", c.badges());
    }

    #[test]
    fn a_priced_model_is_not_free() {
        let c = card(None, r#","cost":{"input":2.1,"output":6.6}"#);
        assert!(!c.free);
    }

    /// The provider's live quote wins over whatever the catalog was built with.
    #[test]
    fn the_provider_price_overrides_the_catalog() {
        let now_free = card(Some(true), r#","cost":{"input":2.1,"output":6.6}"#);
        assert!(now_free.free, "catalog says paid, provider says free");

        let now_paid = card(Some(false), r#","cost":{"input":0,"output":0}"#);
        assert!(!now_paid.free, "catalog says free, provider says paid");
    }

    /// A model the catalog has never heard of can still be marked free.
    #[test]
    fn an_unenriched_model_can_still_be_free() {
        let cards = enrich_models(
            &[RemoteModel {
                id: "brand/new".into(),
                name: None,
                free: Some(true),
            }],
            None,
            None,
        );
        assert!(cards[0].free);
        assert!(cards[0].badges().contains("FREE"), "{}", cards[0].badges());
    }

    #[test]
    fn catalog_fills_in_models_the_listing_skipped() {
        let cat = parse_models_dev_json(
            r#"{
          "xai": {
            "id": "xai",
            "models": {
              "grok-4.6": { "id": "grok-4.6", "name": "Grok 4.6" },
              "grok-4.5": { "id": "grok-4.5", "name": "Grok 4.5" }
            }
          }
        }"#,
        )
        .unwrap();
        let merged = merge_catalog_models(
            vec![RemoteModel {
                id: "grok-4.5".into(),
                name: Some("from-api".into()),
                free: None,
            }],
            Some(&cat),
            Some("xai"),
        );
        let ids: Vec<_> = merged.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"grok-4.5"));
        assert!(ids.contains(&"grok-4.6"));
        let listed = merged.iter().find(|m| m.id == "grok-4.5").unwrap();
        assert_eq!(listed.name.as_deref(), Some("from-api"));
    }
}
