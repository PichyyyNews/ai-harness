pub mod agent;
pub mod constraint_guard;
pub mod long_term;
pub mod mid_term;
pub mod observability;
pub mod short_term;
pub mod worker;

use crate::sessions::store;
use mid_term::SessionMemory;
use tauri::AppHandle;

#[allow(dead_code)]
pub struct TieredMemoryPrompts {
    pub primary: Option<String>,
    pub reminder: Option<String>,
    pub enforced_constraints: Vec<String>,
    pub layer_counts: MemoryLayerCounts,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryLayerCounts {
    pub active_constraints: usize,
    pub mid_term_items: usize,
    pub long_term_facts: usize,
}

pub fn assemble_tiered_memory_prompts(
    app: &AppHandle,
    session_id: &str,
    current_user_message: &str,
    embedding_endpoint: Option<&str>,
) -> TieredMemoryPrompts {
    // 1. Short-Term Constraints
    let active_constraints = short_term::active_constraint_texts(app, session_id);
    let short_term = short_term::active_constraints_prompt(app, session_id);

    // 2. Mid-Term Session Memory
    let (mid_term, mid_term_items) = if let Ok(detail) = store::get(app, session_id) {
        let memory = SessionMemory::from_json_or_prose(&detail.conversation_memory);
        let item_count = memory.goals.len() + memory.decisions.len() + memory.plan_steps.len();
        (memory.formatted_prompt(), item_count)
    } else {
        (None, 0)
    };

    // 3. Long-Term Personalization Facts
    let relevant_facts =
        long_term::retrieve_relevant_facts(app, current_user_message, embedding_endpoint);
    let durable_preferences = relevant_facts
        .iter()
        .filter(|fact| {
            matches!(
                fact.category,
                long_term::FactCategory::Preference | long_term::FactCategory::CommunicationStyle
            )
        })
        .map(|fact| fact.content.clone())
        .collect::<Vec<_>>();
    let long_term = long_term::format_long_term_prompt(&relevant_facts);
    let long_term_count = relevant_facts.len();

    // Constraints and durable communication preferences stay nearest the top
    // of the protected block; mid-term project detail is lower priority.
    // 4. Cross-Session History RAG (Search messages across past sessions)
    let cross_matches = store::search_cross_session_messages(app, session_id, current_user_message, 4);
    let cross_session_prompt = if !cross_matches.is_empty() {
        let mut lines = vec!["[Relevant Past Conversations Across Sessions]".to_string()];
        for m in cross_matches {
            lines.push(format!("- Session \"{}\": User asked: \"{}\"", m.session_title, m.user_content));
            if let Some(reply) = m.assistant_content {
                let snippet = reply.chars().take(200).collect::<String>();
                lines.push(format!("  Assistant replied: \"{snippet}\""));
            }
        }
        Some(lines.join("\n"))
    } else {
        None
    };

    let primary = compose_primary_prompt(short_term, long_term, mid_term, cross_session_prompt);
    let mut enforced_constraints = active_constraints.clone();
    for preference in durable_preferences {
        if !enforced_constraints.contains(&preference) {
            enforced_constraints.push(preference);
        }
    }
    let reminder = short_term::active_constraints_reminder(&enforced_constraints);
    let layer_counts = MemoryLayerCounts {
        active_constraints: active_constraints.len(),
        mid_term_items,
        long_term_facts: long_term_count,
    };

    TieredMemoryPrompts {
        primary,
        reminder,
        enforced_constraints,
        layer_counts,
    }
}

fn compose_primary_prompt(
    short_term: Option<String>,
    long_term: Option<String>,
    mid_term: Option<String>,
    cross_session: Option<String>,
) -> Option<String> {
    let body = [short_term, long_term, mid_term, cross_session]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n\n");
    (!body.trim().is_empty()).then(|| {
        format!(
            "[Memory Directives & Personalization Context]\nActive user profile, preferences, and relevant past history retrieved from local database:\n{body}"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_memory_changes_the_prompt_and_prioritizes_constraints() {
        let without_memory = compose_primary_prompt(None, None, None, None);
        let with_memory = compose_primary_prompt(
            Some("Active constraints: answer in Thai".to_string()),
            Some("Communication style: no emoji".to_string()),
            Some("Goal: finish the memory pipeline".to_string()),
            None,
        )
        .expect("memory prompt");
        assert!(without_memory.is_none());
        assert!(with_memory.starts_with("[Memory Directives"));
        assert!(with_memory.find("answer in Thai") < with_memory.find("Goal:"));
    }
}
