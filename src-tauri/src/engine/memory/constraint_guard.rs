/// Model-backed constraint verification supports any language understood by the
/// local classifier and replaces language-specific post-generation checks.
pub fn violations(endpoint: &str, response: &str, constraints: &[String]) -> Vec<String> {
    let Some(indexes) =
        crate::language_classifier::violated_constraint_indexes(endpoint, response, constraints)
    else {
        // A failed verifier must not invent a violation or trigger a rewrite.
        return Vec::new();
    };
    indexes
        .into_iter()
        .filter_map(|index| constraints.get(index).cloned())
        .collect()
}
