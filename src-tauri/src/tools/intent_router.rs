use tauri::AppHandle;

#[derive(Debug, Clone)]
pub struct AutoToolResult {
    pub tool_name: String,
    pub output: String,
}

/// Language-agnostic intent router for deterministic operations (e.g. pure math formulas).
/// All conceptual tool calls (chat history, system status, model search, workspace files)
/// are selected dynamically by the structured tool router across all languages.
pub fn auto_route_user_intent(_app: &AppHandle, user_text: &str) -> Option<AutoToolResult> {
    let text = user_text.trim();

    // Pure language-agnostic math formula evaluation (e.g. "345 * 12.5 / 100" or "calc 50 + 20")
    if is_pure_math_expression(text) {
        let clean_expr = text
            .trim_start_matches(|c: char| c.is_alphabetic() || c.is_whitespace())
            .trim_end_matches('?')
            .trim();

        if let Ok(output) = super::evaluator::evaluate_expression(clean_expr) {
            return Some(AutoToolResult {
                tool_name: "evaluate_expression".to_string(),
                output,
            });
        }
    }

    None
}

fn is_pure_math_expression(text: &str) -> bool {
    let has_digits = text.chars().any(|c| c.is_ascii_digit());
    let has_math_op = text.chars().any(|c| c == '+' || c == '-' || c == '*' || c == '/' || c == '^');
    has_digits && has_math_op
}
