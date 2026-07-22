use crate::web_search::{ProviderKind, SourceHint};

/// Topic routing belongs to the model-backed retrieval planner. Its local
/// fallback deliberately avoids string matching by language or subject.
pub fn classify(_text: &str) -> SourceHint {
    SourceHint::GeneralWeb
}

pub fn candidates(_text: &str, _hint: &SourceHint) -> Vec<ProviderKind> {
    vec![ProviderKind::GeneralWeb]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_never_classifies_by_user_language() {
        for query in ["latest headlines", "สรุปข่าวเด่นวันนี้หน่อย", "最新ニュース"]
        {
            assert!(matches!(classify(query), SourceHint::GeneralWeb));
            assert_eq!(
                candidates(query, &classify(query)),
                vec![ProviderKind::GeneralWeb]
            );
        }
    }
}
