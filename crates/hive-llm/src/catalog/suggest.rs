//! Heuristic model pick from enriched cards.

use super::enrich::ModelCard;

/// Suggest a single everyday model from the listed cards.
pub fn suggest_model(cards: &[ModelCard]) -> Option<String> {
    if cards.is_empty() {
        return None;
    }

    cards
        .iter()
        .find(|c| {
            let id = c.id.to_ascii_lowercase();
            id.contains("fast") || id.contains("flash") || id.contains("sonnet")
        })
        .map(|c| c.id.clone())
        .or_else(|| {
            cards
                .iter()
                .filter(|c| c.tools || c.reasoning || c.context >= 100_000)
                .max_by_key(|c| (c.reasoning as u8, c.context, c.tools as u8))
                .map(|c| c.id.clone())
        })
        .or_else(|| cards.iter().max_by_key(|c| c.context).map(|c| c.id.clone()))
        .or_else(|| cards.first().map(|c| c.id.clone()))
}

/// Back-compat alias used by older call sites.
#[derive(Debug, Clone, Default)]
pub struct RolePicks {
    pub default: Option<String>,
}

/// Suggest a default model (same as [`suggest_model`]).
pub fn suggest_roles(cards: &[ModelCard]) -> RolePicks {
    RolePicks {
        default: suggest_model(cards),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, vision: bool, context: u64, reasoning: bool) -> ModelCard {
        ModelCard {
            id: id.into(),
            name: id.into(),
            vision,
            video: false,
            audio: false,
            reasoning,
            tools: true,
            context,
            enriched: true,
            cost_input: 0.0,
            cost_output: 0.0,
        }
    }

    #[test]
    fn prefers_fast_flash() {
        let cards = vec![
            card("big-smart", false, 200_000, true),
            card("tiny-flash", false, 32_000, false),
            card("see-me", true, 128_000, false),
        ];
        assert_eq!(suggest_model(&cards).as_deref(), Some("tiny-flash"));
    }
}
