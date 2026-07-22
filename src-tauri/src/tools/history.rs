use crate::sessions::store;
use serde_json::json;
use tauri::AppHandle;

pub fn search_chat_history(app: &AppHandle, query: &str, limit: usize) -> Result<String, String> {
    let clean_query = query.trim();
    if clean_query.is_empty() {
        return Err("Search query cannot be empty.".to_string());
    }

    let sessions = store::list(app, None)?;
    let mut matches = Vec::new();
    let query_lower = clean_query.to_lowercase();

    for session in sessions {
        if let Ok(detail) = store::get(app, &session.id) {
            let matched_messages: Vec<_> = detail
                .messages
                .iter()
                .filter(|msg| msg.content.to_lowercase().contains(&query_lower))
                .take(3)
                .map(|msg| {
                    json!({
                        "role": msg.role,
                        "snippet": msg.content.chars().take(200).collect::<String>(),
                        "createdAt": msg.created_at
                    })
                })
                .collect();

            if !matched_messages.is_empty()
                || session.title.to_lowercase().contains(&query_lower)
            {
                matches.push(json!({
                    "sessionId": session.id,
                    "title": session.title,
                    "updatedAt": session.updated_at,
                    "matchedMessages": matched_messages
                }));
            }
        }

        if matches.len() >= limit.max(1).min(20) {
            break;
        }
    }

    if matches.is_empty() {
        Ok(json!({
            "query": clean_query,
            "count": 0,
            "results": [],
            "message": format!("No past chat sessions found matching '{clean_query}'.")
        })
        .to_string())
    } else {
        Ok(json!({
            "query": clean_query,
            "count": matches.len(),
            "results": matches
        })
        .to_string())
    }
}

pub fn get_session_details(app: &AppHandle, session_id: &str) -> Result<String, String> {
    let clean_id = session_id.trim();

    let target_id = if clean_id.eq_ignore_ascii_case("latest")
        || clean_id.eq_ignore_ascii_case("previous")
        || clean_id.eq_ignore_ascii_case("last")
    {
        let sessions = store::list(app, None)?;
        if sessions.len() > 1 {
            sessions[1].id.clone()
        } else if let Some(first) = sessions.first() {
            first.id.clone()
        } else {
            return Err("No past sessions found in database.".to_string());
        }
    } else {
        clean_id.to_string()
    };

    let detail = store::get(app, &target_id)?;
    let messages: Vec<_> = detail
        .messages
        .into_iter()
        .map(|msg| {
            json!({
                "role": msg.role,
                "content": msg.content,
                "createdAt": msg.created_at
            })
        })
        .collect();

    Ok(json!({
        "sessionId": detail.session.id,
        "title": detail.session.title,
        "createdAt": detail.session.created_at,
        "updatedAt": detail.session.updated_at,
        "messageCount": messages.len(),
        "messages": messages
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_normalization_works() {
        assert_eq!("  test query  ".trim(), "test query");
    }
}
