use serde_json::json;

pub fn evaluate_expression(expr: &str) -> Result<String, String> {
    let clean = expr.trim();
    if clean.is_empty() {
        return Err("Expression cannot be empty.".to_string());
    }

    // Basic safe mathematical evaluation without arbitrary code execution
    let result = meval::eval_str(clean).map_err(|e| format!("Math evaluation error: {e}"))?;

    Ok(json!({
        "expression": clean,
        "result": result
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_math_expressions_correctly() {
        let res = evaluate_expression("((345 * 12.5) / 100) * 1.07").unwrap();
        assert!(res.contains("46.14375"));
    }
}
