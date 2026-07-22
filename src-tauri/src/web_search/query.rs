use crate::{
    language_classifier::MessageClassification,
    web_search::{source_router, QueryPlan, SubQuestion},
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingDecision {
    Search { query: String, reason: &'static str },
    Skip { reason: &'static str },
}

/// Search-first routing for a local model. Local models should not be asked to
/// silently rely on stale training data for ordinary factual questions, so the
/// only offline paths are clearly self-contained writing or acknowledgement
/// turns.
pub fn search_query_for(
    message: &str,
    classification: Option<&MessageClassification>,
) -> Option<String> {
    match routing_decision(message, classification) {
        RoutingDecision::Search { query, .. } => Some(query),
        RoutingDecision::Skip { .. } => None,
    }
}

pub fn routing_decision(
    message: &str,
    classification: Option<&MessageClassification>,
) -> RoutingDecision {
    let query = message.trim().trim_start_matches("search:").trim();
    if query.len() < 3 {
        return RoutingDecision::Skip {
            reason: "query_too_short",
        };
    }
    if query.len() > 1_500 {
        return RoutingDecision::Skip {
            reason: "query_too_long",
        };
    }
    if is_personal_or_memory_query(query) {
        return RoutingDecision::Skip {
            reason: "personal_memory_query",
        };
    }
    if let Some(classification) = classification {
        if !classification.needs_search {
            return RoutingDecision::Skip {
                reason: "semantic_classifier_no_search",
            };
        }
    }
    RoutingDecision::Search {
        query: query.to_string(),
        reason: "search_first_default",
    }
}

pub fn plan_query(
    user_query: &str,
    classification: Option<&MessageClassification>,
) -> Option<QueryPlan> {
    let clean_query = search_query_for(user_query, classification)?;

    // Keep the original request intact. English-only conjunction parsing used
    // to split only some languages and could change meaning for the rest.
    // The model-backed provider planner receives the complete request.
    let sub_questions = vec![SubQuestion {
        id: Uuid::new_v4(),
        text: clean_query.clone(),
        source_hint: source_router::classify(&clean_query),
        depends_on: None,
    }];
    Some(QueryPlan {
        original_query: clean_query,
        sub_questions,
        is_compound: false,
    })
}

fn is_personal_or_memory_query(text: &str) -> bool {
    let lower = text.to_lowercase();
    let keywords = [
        "ผมคือใคร", "ผมชื่ออะไร", "ฉันคือใคร", "ฉันชื่ออะไร", "จำผมได้ไหม",
        "จำฉันได้ไหม", "เราเคยคุย", "เคยคุยอะไร", "ผมทำงานอะไร", "ใครคือผม",
        "who am i", "what is my name", "do you remember me", "my role is"
    ];
    keywords.iter().any(|k| lower.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classification(needs_search: bool) -> MessageClassification {
        MessageClassification {
            needs_search,
            is_constraint: false,
            constraint_text: None,
            scope: None,
        }
    }

    #[test]
    fn routes_ordinary_factual_prompts_to_search() {
        let classifier = classification(true);
        assert!(search_query_for("Explain Rust ownership", Some(&classifier)).is_some());
        assert!(search_query_for("How do I cook pad thai?", Some(&classifier)).is_some());
    }

    #[test]
    fn follows_a_language_agnostic_no_search_classification() {
        let classifier = classification(false);
        for message in ["thanks", "สวัสดี", "こんにちは", "hola", "مرحبا"] {
            assert!(
                search_query_for(message, Some(&classifier)).is_none(),
                "{message}"
            );
        }
    }

    #[test]
    fn routes_thai_current_news_to_search() {
        let classifier = classification(true);
        let decision = routing_decision("วันนี้มีข่าวอะไรน่าสนใจ", Some(&classifier));
        assert!(matches!(decision, RoutingDecision::Search { .. }));
    }

    #[test]
    fn keeps_compound_queries_intact_for_model_planning() {
        let classifier = classification(true);
        let plan = plan_query(
            "What is the weather in Chiang Mai and who is the mayor?",
            Some(&classifier),
        )
        .unwrap();
        assert!(!plan.is_compound);
        assert_eq!(plan.sub_questions.len(), 1);
    }
}
