use crate::web_search::Grounding;

const CLAIM_SUPPORT_THRESHOLD: f32 = 0.45;

#[allow(dead_code)]
pub struct Claim {
    pub text: String,
    pub flagged: bool,
}

pub fn check_faithfulness(response: &str, grounding: &Grounding) -> (Vec<Claim>, bool) {
    let claims = extract_atomic_claims(response);
    let mut flagged_any = false;
    let mut result_claims = Vec::new();

    for claim_text in claims {
        let support = max_support_score(&claim_text, &grounding.prompt);
        let flagged = support < CLAIM_SUPPORT_THRESHOLD;
        if flagged {
            flagged_any = true;
        }
        result_claims.push(Claim {
            text: claim_text,
            flagged,
        });
    }

    (result_claims, flagged_any)
}

fn extract_atomic_claims(response: &str) -> Vec<String> {
    response
        .split(|c| c == '.' || c == '!' || c == '?')
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() > 10)
        .collect()
}

fn max_support_score(claim: &str, evidence_prompt: &str) -> f32 {
    let claim_terms: std::collections::HashSet<_> = claim
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .map(String::from)
        .collect();

    if claim_terms.is_empty() {
        return 1.0;
    }

    let evidence_terms: std::collections::HashSet<_> = evidence_prompt
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .map(String::from)
        .collect();

    let matched = claim_terms
        .iter()
        .filter(|t| evidence_terms.contains(*t))
        .count();

    (matched as f32) / (claim_terms.len() as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_and_unsupported_claims() {
        let grounding = Grounding {
            sources: vec![],
            prompt: "The Eiffel Tower in Paris was completed in 1889.".to_string(),
            retrieval_trace: Vec::new(),
        };
        let response = "The Eiffel Tower was completed in 1889. It is painted bright green.";
        let (claims, flagged) = check_faithfulness(response, &grounding);
        assert!(flagged);
        assert_eq!(claims.len(), 2);
    }
}
