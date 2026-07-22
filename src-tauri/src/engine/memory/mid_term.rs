use super::worker::{ExtractedDecision, ExtractedGoal, ExtractedPlanStep};
use crate::sessions::store;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionMemory {
    pub goals: Vec<Goal>,
    pub decisions: Vec<Decision>,
    pub open_questions: Vec<String>,
    pub plan_steps: Vec<PlanStep>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub description: String,
    pub status: GoalStatus,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Achieved,
    Abandoned,
}

impl GoalStatus {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "achieved" => GoalStatus::Achieved,
            "abandoned" => GoalStatus::Abandoned,
            _ => GoalStatus::Active,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub what: String,
    pub why: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub description: String,
    pub status: StepStatus,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Done,
}

impl StepStatus {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "in_progress" | "inprogress" => StepStatus::InProgress,
            "done" | "completed" => StepStatus::Done,
            _ => StepStatus::Pending,
        }
    }
}

impl SessionMemory {
    pub fn from_json_or_prose(raw: &str) -> Self {
        if raw.trim().starts_with('{') {
            serde_json::from_str::<SessionMemory>(raw).unwrap_or_default()
        } else if !raw.trim().is_empty() {
            SessionMemory {
                goals: vec![Goal {
                    description: raw.to_string(),
                    status: GoalStatus::Active,
                }],
                ..Default::default()
            }
        } else {
            SessionMemory::default()
        }
    }

    #[allow(dead_code)]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn formatted_prompt(&self) -> Option<String> {
        let mut lines = Vec::new();

        if !self.goals.is_empty() {
            let goals_str: Vec<String> = self
                .goals
                .iter()
                .filter(|g| g.status == GoalStatus::Active)
                .map(|g| format!("- Goal: {}", g.description))
                .collect();
            if !goals_str.is_empty() {
                lines.extend(goals_str);
            }
        }

        if !self.decisions.is_empty() {
            let decisions_str: Vec<String> = self
                .decisions
                .iter()
                .map(|d| {
                    if let Some(why) = &d.why {
                        format!("- Decision: {} ({why})", d.what)
                    } else {
                        format!("- Decision: {}", d.what)
                    }
                })
                .collect();
            lines.extend(decisions_str);
        }

        if !self.plan_steps.is_empty() {
            let steps_str: Vec<String> = self
                .plan_steps
                .iter()
                .map(|s| format!("- Step [{:?}]: {}", s.status, s.description))
                .collect();
            lines.extend(steps_str);
        }

        if lines.is_empty() {
            None
        } else {
            Some(format!(
                "Session Context & Progress:\n{}\n",
                lines.join("\n")
            ))
        }
    }
}

pub fn merge_extracted_memory(
    app: &AppHandle,
    session_id: &str,
    new_goals: &[ExtractedGoal],
    new_decisions: &[ExtractedDecision],
    new_steps: &[ExtractedPlanStep],
) {
    let mut memory = match store::get(app, session_id) {
        Ok(detail) => SessionMemory::from_json_or_prose(&detail.conversation_memory),
        Err(_) => SessionMemory::default(),
    };

    // 1. Merge goals
    for g in new_goals {
        let desc = g.description.trim();
        if desc.is_empty() {
            continue;
        }
        if let Some(existing) = memory
            .goals
            .iter_mut()
            .find(|item| similarity(&item.description, desc) > 0.6)
        {
            existing.status = GoalStatus::from_str(&g.status);
        } else {
            memory.goals.push(Goal {
                description: desc.to_string(),
                status: GoalStatus::from_str(&g.status),
            });
        }
    }

    // 2. Merge decisions
    for d in new_decisions {
        let what = d.what.trim();
        if what.is_empty() {
            continue;
        }
        if !memory
            .decisions
            .iter()
            .any(|item| similarity(&item.what, what) > 0.7)
        {
            memory.decisions.push(Decision {
                what: what.to_string(),
                why: d.why.clone(),
            });
        }
    }

    // 3. Merge plan steps
    for s in new_steps {
        let desc = s.description.trim();
        if desc.is_empty() {
            continue;
        }
        if let Some(existing) = memory
            .plan_steps
            .iter_mut()
            .find(|item| similarity(&item.description, desc) > 0.6)
        {
            existing.status = StepStatus::from_str(&s.status);
        } else {
            memory.plan_steps.push(PlanStep {
                description: desc.to_string(),
                status: StepStatus::from_str(&s.status),
            });
        }
    }

    let json = memory.to_json();
    let _ = store::set_memory(app, session_id, &json);
    eprintln!("[memory-worker] merged mid-term memory for session {session_id}");
}

fn similarity(s1: &str, s2: &str) -> f32 {
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
    fn serializes_and_formats_session_memory() {
        let memory = SessionMemory {
            goals: vec![Goal {
                description: "Build Tiered Memory".to_string(),
                status: GoalStatus::Active,
            }],
            decisions: vec![Decision {
                what: "Use SQLite".to_string(),
                why: Some("Fast & local".to_string()),
            }],
            open_questions: vec![],
            plan_steps: vec![PlanStep {
                description: "Implement short_term".to_string(),
                status: StepStatus::Done,
            }],
        };

        let json = memory.to_json();
        let parsed = SessionMemory::from_json_or_prose(&json);
        assert_eq!(parsed.goals.len(), 1);

        let prompt = memory.formatted_prompt().unwrap();
        assert!(prompt.contains("Build Tiered Memory"));
        assert!(prompt.contains("Use SQLite"));
    }
}
