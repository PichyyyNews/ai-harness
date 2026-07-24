use std::time::Duration;

/// Background Prompt Enhancer:
/// Takes the raw user input and recent conversation context, then asks the local
/// engine for a quick structured enhancement. Runs transparently in the background.
pub fn enhance_prompt(endpoint: &str, raw_input: &str, context: &str) -> String {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return raw_input.to_string();
    }

    // Direct greetings pass through unchanged to preserve natural conversational speed
    let lower = trimmed.to_lowercase();
    if lower == "สวัสดี" || lower == "สวัสดีครับ" || lower == "สวัสดีค่ะ" || lower == "hello" || lower == "hi" {
        return raw_input.to_string();
    }

    let prompt = format!(
        r#"You are a background prompt optimizer for an AI assistant system.
Your job is to clarify the user's raw message into a structured, clear intent statement for system routing and reasoning.

Rules:
1. Preserve the user's original language (e.g. Thai or English) and core intent.
2. If the user input is broad or underspecified (e.g. "หาข่าวให้หน่อย", "เขียนโค้ดให้หน่อย"), state what the user wants and note that category/scope details are needed.
3. Output ONLY the optimized intent statement in 1-2 sentences. No conversational filler or thinking prose.

Context:
{context}

Raw user input:
"{trimmed}"

Optimized Intent Statement:"#
    );

    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
    else {
        return raw_input.to_string();
    };

    match client
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&serde_json::json!({
            "messages": [{"role":"user","content":prompt}],
            "max_tokens": 96,
            "temperature": 0.1,
            "stream": false,
            "chat_template_kwargs": {"enable_thinking": false}
        }))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(|res| res.json::<serde_json::Value>())
    {
        Ok(res) => {
            if let Some(content) = res["choices"][0]["message"]["content"].as_str() {
                let cleaned = content
                    .lines()
                    .filter(|l| !l.trim().starts_with("```") && !l.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string();
                if !cleaned.is_empty() {
                    return cleaned;
                }
            }
            raw_input.to_string()
        }
        Err(err) => {
            eprintln!("[prompt-enhancer] background pass skipped: {err}");
            raw_input.to_string()
        }
    }
}
