//! Adaptive multi-pass reasoning loop for frontier-grade response verification.
//!
//! This module evaluates evidence sufficiency after the initial retrieval pass
//! and decides whether to:
//! 1. Accept the evidence and proceed to final synthesis (fast path).
//! 2. Run a targeted secondary search batch to fill knowledge gaps.
//! 3. Signal the frontend to display an interactive choice UI for the user.

use super::{
    EvidenceChunk, EvidenceQuality, RawEvidence, RetrievalTraceEntry,
    RetrievalTraceRecorder, SourceKind, SubQuestion, SubQuestionResult,
};

/// Minimum combined confidence to skip the secondary pass.
const SUFFICIENCY_THRESHOLD: f32 = 0.65;

/// Maximum number of reasoning passes before forcing final synthesis.
const MAX_PASSES: usize = 3;

/// Describes what action the reasoning loop recommends after evaluating evidence.
#[derive(Debug, Clone)]
pub enum ReasoningAction {
    /// Evidence is strong enough; proceed directly to final answer.
    Synthesize,
    /// Evidence has quantitative gaps; run a targeted secondary search.
    RefineSearch {
        refined_queries: Vec<String>,
        gap_description: String,
    },
    /// The query is open-ended with multiple valid directions; ask the user.
    AskUser {
        question: String,
        options: Vec<String>,
    },
}

/// Result of a single reasoning pass evaluation.
#[derive(Debug, Clone)]
pub struct PassResult {
    pub pass_number: usize,
    pub action: ReasoningAction,
    pub confidence: f32,
    pub status_message: String,
}

/// Evaluates evidence sufficiency and determines the next action.
///
/// This is the core decision function called after each retrieval pass.
pub fn evaluate_pass(
    pass_number: usize,
    sub_results: &[SubQuestionResult],
    original_query: &str,
) -> PassResult {
    if pass_number >= MAX_PASSES {
        return PassResult {
            pass_number,
            action: ReasoningAction::Synthesize,
            confidence: aggregate_confidence(sub_results),
            status_message: format!("Pass {pass_number}: Maximum reasoning depth reached; synthesizing final answer"),
        };
    }

    let confidence = aggregate_confidence(sub_results);
    let total_chunks: usize = sub_results.iter().map(|r| r.evidence.chunks.len()).sum();
    let has_weak = sub_results.iter().any(|r| r.quality == EvidenceQuality::Weak);
    let all_empty = sub_results.iter().all(|r| r.evidence.chunks.is_empty());

    // Fast path: strong evidence across all sub-questions
    if confidence >= SUFFICIENCY_THRESHOLD && !has_weak && total_chunks >= 2 {
        return PassResult {
            pass_number,
            action: ReasoningAction::Synthesize,
            confidence,
            status_message: format!(
                "Pass {pass_number}: Evidence confidence {:.0}% — sufficient for final answer",
                confidence * 100.0
            ),
        };
    }

    // If all evidence is empty, we can't refine — just synthesize with what we have
    if all_empty {
        return PassResult {
            pass_number,
            action: ReasoningAction::Synthesize,
            confidence,
            status_message: format!("Pass {pass_number}: No evidence retrieved; synthesizing best-effort answer"),
        };
    }

    // Gap analysis: identify weak sub-questions that need targeted search
    let weak_topics: Vec<String> = sub_results
        .iter()
        .filter(|r| r.quality == EvidenceQuality::Weak || r.evidence.chunks.is_empty())
        .map(|r| r.sub_q.text.clone())
        .collect();

    if !weak_topics.is_empty() {
        let gap_desc = format!(
            "Missing or weak evidence for: {}",
            weak_topics.join(", ")
        );
        return PassResult {
            pass_number,
            action: ReasoningAction::RefineSearch {
                refined_queries: weak_topics,
                gap_description: gap_desc.clone(),
            },
            confidence,
            status_message: format!(
                "Pass {pass_number}: Evidence confidence {:.0}% — {gap_desc}",
                confidence * 100.0
            ),
        };
    }

    // Default: confidence is below threshold but no specific gaps identified
    PassResult {
        pass_number,
        action: ReasoningAction::Synthesize,
        confidence,
        status_message: format!(
            "Pass {pass_number}: Evidence confidence {:.0}% — proceeding with available evidence",
            confidence * 100.0
        ),
    }
}

/// Computes the aggregate confidence across all sub-question results.
fn aggregate_confidence(sub_results: &[SubQuestionResult]) -> f32 {
    if sub_results.is_empty() {
        return 0.0;
    }
    let sum: f32 = sub_results.iter().map(|r| r.confidence.combined).sum();
    sum / sub_results.len() as f32
}

/// Records a reasoning pass into the retrieval trace for UI visualization.
pub fn record_pass_trace(
    trace: &RetrievalTraceRecorder,
    pass_result: &PassResult,
) {
    let action_label = match &pass_result.action {
        ReasoningAction::Synthesize => "proceeding to final synthesis".to_string(),
        ReasoningAction::RefineSearch { gap_description, .. } => {
            format!("requesting targeted search: {gap_description}")
        }
        ReasoningAction::AskUser { question, .. } => {
            format!("requesting user input: {question}")
        }
    };

    trace.record(RetrievalTraceEntry {
        stage: format!("reasoning pass {}", pass_result.pass_number),
        provider: "Adaptive Reasoning Engine".to_string(),
        endpoint: None,
        title: Some(pass_result.status_message.clone()),
        url: None,
        preview: None,
        score: Some(pass_result.confidence as f64),
        decision: action_label,
        detail: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_search::{SourceHint};
    use uuid::Uuid;

    fn make_sub_result(text: &str, quality: EvidenceQuality, confidence: f32, chunk_count: usize) -> SubQuestionResult {
        let chunks: Vec<EvidenceChunk> = (0..chunk_count)
            .map(|i| EvidenceChunk {
                text: format!("Evidence chunk {i} for {text}"),
                source_url: format!("https://example.com/{i}"),
                source_title: format!("Source {i}"),
                host: "example.com".to_string(),
            })
            .collect();
        SubQuestionResult {
            sub_q: SubQuestion {
                id: Uuid::new_v4(),
                text: text.to_string(),
                source_hint: SourceHint::GeneralWeb,
                depends_on: None,
            },
            evidence: RawEvidence {
                chunks,
                source_kind: SourceKind::Web,
            },
            confidence: Confidence {
                relevance: confidence,
                agreement: 0.5,
                coverage: confidence,
                combined: confidence,
            },
            quality,
        }
    }

    #[test]
    fn strong_evidence_triggers_synthesis() {
        let results = vec![
            make_sub_result("test query", EvidenceQuality::Strong, 0.85, 3),
        ];
        let pass = evaluate_pass(1, &results, "test query");
        assert!(matches!(pass.action, ReasoningAction::Synthesize));
        assert!(pass.confidence >= SUFFICIENCY_THRESHOLD);
    }

    #[test]
    fn weak_evidence_triggers_refine_search() {
        let results = vec![
            make_sub_result("strong topic", EvidenceQuality::Strong, 0.80, 2),
            make_sub_result("weak topic", EvidenceQuality::Weak, 0.30, 1),
        ];
        let pass = evaluate_pass(1, &results, "test query");
        assert!(matches!(pass.action, ReasoningAction::RefineSearch { .. }));
    }

    #[test]
    fn max_passes_forces_synthesis() {
        let results = vec![
            make_sub_result("weak topic", EvidenceQuality::Weak, 0.20, 1),
        ];
        let pass = evaluate_pass(MAX_PASSES, &results, "test query");
        assert!(matches!(pass.action, ReasoningAction::Synthesize));
    }

    #[test]
    fn empty_evidence_synthesizes_best_effort() {
        let results = vec![
            make_sub_result("empty topic", EvidenceQuality::Weak, 0.0, 0),
        ];
        let pass = evaluate_pass(1, &results, "test query");
        assert!(matches!(pass.action, ReasoningAction::Synthesize));
    }
}
