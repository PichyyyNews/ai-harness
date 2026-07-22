use super::worker::ExtractedFact;
use crate::sessions::store;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactCategory {
    Preference,
    CommunicationStyle,
    RecurringProject,
    RecurringTopic,
    SkillLevel,
}

impl FactCategory {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            FactCategory::Preference => "preference",
            FactCategory::CommunicationStyle => "communication_style",
            FactCategory::RecurringProject => "recurring_project",
            FactCategory::RecurringTopic => "recurring_topic",
            FactCategory::SkillLevel => "skill_level",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "preference" => FactCategory::Preference,
            "communication_style" | "communicationstyle" => FactCategory::CommunicationStyle,
            "recurring_project" | "recurringproject" => FactCategory::RecurringProject,
            "recurring_topic" | "recurringtopic" => FactCategory::RecurringTopic,
            "skill_level" | "skilllevel" => FactCategory::SkillLevel,
            _ => FactCategory::Preference,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LongTermFact {
    pub id: String,
    pub category: FactCategory,
    pub content: String,
    pub source_session_id: Option<String>,
    pub confidence: f32,
    pub last_confirmed_at: String,
}

pub fn is_allowed_fact(content: &str) -> bool {
    let lower = content.to_lowercase();
    let prohibited = [
        "medical",
        "disease",
        "doctor",
        "health",
        "symptom",
        "politic",
        "election",
        "democrat",
        "republican",
        "religion",
        "god",
        "church",
        "buddhist",
        "christian",
        "muslim",
        "password",
        "secret",
        "api_key",
    ];

    !prohibited.iter().any(|p| lower.contains(p))
}

pub fn process_extracted_facts(
    app: &AppHandle,
    embedding_endpoint: Option<&str>,
    session_id: &str,
    facts: &[ExtractedFact],
) -> usize {
    let mut stored = 0;
    for f in facts {
        let content = f.content.trim();
        // The multilingual extraction model performs the primary sensitive-
        // category decision. This deterministic check remains a conservative
        // English failsafe rather than pretending raw cosine scores are a
        // reliable safety classifier across every script.
        if content.is_empty() || !is_allowed_fact(content) {
            eprintln!("[memory-worker] fact rejected (empty or restricted topic): '{content}'");
            continue;
        }

        let category = FactCategory::from_str(&f.category);
        let confidence = f.confidence.clamp(0.1, 1.0);

        if let Ok(existing) = store::get_all_long_term_facts(app) {
            let mut updated_existing = false;
            for (id, cat, old_content, _, _, _) in existing {
                if cat == category.as_str()
                    && embedding_endpoint
                        .and_then(|endpoint| semantic_similarity(endpoint, content, &old_content))
                        .unwrap_or_else(|| lexical_similarity(content, &old_content))
                        > 0.72
                {
                    let new_id = store::save_long_term_fact(
                        app,
                        category.as_str(),
                        content,
                        Some(session_id),
                        confidence,
                    )
                    .unwrap_or_default();
                    let _ = store::supersede_long_term_fact(app, &id, &new_id);
                    eprintln!("[memory-worker] long-term fact updated/superseded: '{old_content}' -> '{content}'");
                    updated_existing = true;
                    stored += 1;
                    break;
                }
            }
            if updated_existing {
                continue;
            }
        }

        if let Ok(new_id) = store::save_long_term_fact(
            app,
            category.as_str(),
            content,
            Some(session_id),
            confidence,
        ) {
            eprintln!(
                "[memory-worker] saved new long-term fact ({new_id}): [{}] '{content}'",
                category.as_str()
            );
            stored += 1;
        }
    }
    stored
}

pub fn retrieve_relevant_facts(
    app: &AppHandle,
    current_prompt: &str,
    embedding_endpoint: Option<&str>,
) -> Vec<LongTermFact> {
    let raw_facts = match store::get_all_long_term_facts(app) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let facts: Vec<LongTermFact> = raw_facts
        .into_iter()
        .map(
            |(id, cat, content, src, confidence, last_confirmed)| LongTermFact {
                id,
                category: FactCategory::from_str(&cat),
                content,
                source_session_id: src,
                confidence,
                last_confirmed_at: last_confirmed,
            },
        )
        .collect();

    let semantic_vectors = embedding_endpoint.and_then(|endpoint| {
        let mut inputs = Vec::with_capacity(facts.len() + 1);
        inputs.push(current_prompt.chars().take(1_500).collect::<String>());
        inputs.extend(
            facts
                .iter()
                .map(|fact| fact.content.chars().take(1_500).collect::<String>()),
        );
        crate::engine::embedding_runtime::embed_retrieval(
            endpoint,
            &inputs[0],
            &inputs[1..],
        )
        .ok()
    });
    let mut selected = Vec::new();
    let mut remainder = Vec::new();

    for (index, fact) in facts.into_iter().enumerate() {
        if fact.category == FactCategory::CommunicationStyle {
            selected.push(fact);
        } else {
            let score = semantic_vectors
                .as_ref()
                .and_then(|vectors| {
                    vectors
                        .first()
                        .zip(vectors.get(index + 1))
                        .map(|(query, fact)| {
                            crate::engine::embedding_runtime::cosine_similarity(query, fact)
                        })
                })
                .unwrap_or_else(|| lexical_similarity(current_prompt, &fact.content));
            let minimum = if semantic_vectors.is_some() {
                0.35
            } else {
                0.15
            };
            if score > minimum {
                remainder.push((fact, score));
            }
        }
    }

    remainder.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_k: Vec<LongTermFact> = remainder.into_iter().take(6).map(|(f, _)| f).collect();
    selected.extend(top_k);
    selected
}

pub fn format_long_term_prompt(facts: &[LongTermFact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }

    let items: Vec<String> = facts
        .iter()
        .map(|f| match f.category {
            FactCategory::Preference | FactCategory::CommunicationStyle => {
                format!(
                    "- You MUST apply this user preference when relevant: {}",
                    f.content
                )
            }
            _ => format!(
                "- Use this established user context when relevant: {}",
                f.content
            ),
        })
        .collect();

    Some(format!("Long-term user memory:\n{}\n", items.join("\n")))
}

fn semantic_similarity(endpoint: &str, left: &str, right: &str) -> Option<f32> {
    let inputs = vec![
        left.chars().take(1_500).collect::<String>(),
        right.chars().take(1_500).collect::<String>(),
    ];
    let vectors =
        crate::engine::embedding_runtime::embed_sentence_similarity(endpoint, &inputs).ok()?;
    Some(crate::engine::embedding_runtime::cosine_similarity(
        vectors.first()?,
        vectors.get(1)?,
    ))
}

fn lexical_similarity(s1: &str, s2: &str) -> f32 {
    let w1: std::collections::HashSet<_> = s1
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .map(String::from)
        .collect();
    let w2: std::collections::HashSet<_> = s2
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .map(String::from)
        .collect();

    if w1.is_empty() || w2.is_empty() {
        return 0.0;
    }

    let common = w1.intersection(&w2).count() as f32;
    let union = w1.union(&w2).count() as f32;
    common / union
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_sensitive_topics() {
        assert!(!is_allowed_fact(
            "User is a registered voter for election party"
        ));
        assert!(is_allowed_fact(
            "User prefers dark mode UI and concise Rust code"
        ));
    }
}
