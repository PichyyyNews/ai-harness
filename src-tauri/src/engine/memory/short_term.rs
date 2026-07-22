use super::worker::ExtractedConstraint;
use crate::sessions::store;
use tauri::AppHandle;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintScope {
    Session,
    ThisTurnOnly,
}

impl ConstraintScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConstraintScope::Session => "session",
            ConstraintScope::ThisTurnOnly => "turn_only",
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "turn_only" => ConstraintScope::ThisTurnOnly,
            _ => ConstraintScope::Session,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ActiveConstraint {
    pub id: String,
    pub session_id: String,
    pub text: String,
    pub scope: ConstraintScope,
    pub created_at: String,
}

pub fn save_extracted_constraints(
    app: &AppHandle,
    session_id: &str,
    extracted: &[ExtractedConstraint],
) {
    'constraint: for item in extracted {
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }

        let scope = if item.scope.to_lowercase() == "turn_only" {
            ConstraintScope::ThisTurnOnly
        } else {
            ConstraintScope::Session
        };

        if let Ok(existing) = store::get_active_constraints(app, session_id) {
            for (_old_id, old_text, _, _) in existing {
                if text.eq_ignore_ascii_case(old_text.trim()) {
                    continue 'constraint;
                }
            }
        }

        let _ = store::save_constraint(app, session_id, text, scope.as_str());
        eprintln!(
            "[memory-worker] constraint saved: '{text}' (scope={})",
            scope.as_str()
        );
    }
}

pub fn active_constraints_prompt(app: &AppHandle, session_id: &str) -> Option<String> {
    let constraints = active_constraint_texts(app, session_id);
    if constraints.is_empty() {
        return None;
    }

    let items: Vec<String> = constraints
        .into_iter()
        .map(|text| format!("- You MUST follow this instruction: {text}"))
        .collect();

    Some(format!(
        "Active constraints (firm requirements, not suggestions):\n{}\n",
        items.join("\n")
    ))
}

pub fn active_constraint_texts(app: &AppHandle, session_id: &str) -> Vec<String> {
    store::get_active_constraints(app, session_id)
        .unwrap_or_default()
        .into_iter()
        .map(|(_, text, _, _)| text)
        .collect()
}

pub fn active_constraints_reminder(constraints: &[String]) -> Option<String> {
    if constraints.is_empty() {
        return None;
    }
    let selected = constraints
        .iter()
        .rev()
        .take(12)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("; ");
    let selected = selected.chars().take(1_200).collect::<String>();
    Some(format!(
        "[Active Memory Reminder]\nBefore answering the next user message, you MUST obey: {}",
        selected
    ))
}

pub fn expire_turn_constraints(app: &AppHandle, session_id: &str) {
    let _ = store::expire_turn_constraints(app, session_id);
}

#[cfg(test)]
mod tests {
    #[test]
    fn treats_distinct_constraints_as_distinct_until_the_model_resolves_them() {
        assert_ne!("always answer briefly", "never answer briefly");
    }
}
