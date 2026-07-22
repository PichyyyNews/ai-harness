/// Search-first routing for a local model. Local models should not be asked to
/// silently rely on stale training data for ordinary factual questions, so the
/// only offline paths are clearly self-contained writing or acknowledgement
/// turns.
pub fn search_query_for(message: &str) -> Option<String> {
    let query = message.trim().trim_start_matches("search:").trim();
    if query.len() < 3
        || query.len() > 1_500
        || explicitly_offline(query)
        || is_self_contained_task(query)
    {
        return None;
    }
    Some(query.to_string())
}

fn explicitly_offline(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "do not search",
        "don't search",
        "without internet",
        "offline only",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn is_self_contained_task(message: &str) -> bool {
    let lower = message.trim().to_lowercase();
    let acknowledgements = [
        "ok",
        "okay",
        "thanks",
        "thank you",
        "yes",
        "no",
        "continue",
        "hi",
        "hello",
    ];
    if acknowledgements.iter().any(|value| lower == *value) {
        return true;
    }

    // Transformations with all source material already in the turn do not
    // benefit from retrieval and should remain private/offline by default.
    let transform = [
        "rewrite",
        "translate",
        "summarize",
        "proofread",
        "fix grammar",
        "write a poem",
        "write a haiku",
    ];
    transform.iter().any(|prefix| lower.starts_with(prefix))
        && message.len() > 80
        && (lower.contains("this ") || lower.contains("following") || lower.contains(':'))
}

#[cfg(test)]
mod tests {
    use super::search_query_for;

    #[test]
    fn routes_ordinary_factual_prompts_to_search() {
        assert!(search_query_for("Explain Rust ownership").is_some());
        assert!(search_query_for("How do I cook pad thai?").is_some());
    }

    #[test]
    fn keeps_explicitly_self_contained_turns_offline() {
        assert!(search_query_for("thanks").is_none());
        assert!(search_query_for("Rewrite this paragraph so it is clear: the source text is already here and long enough to be transformed without outside research.").is_none());
    }
}
