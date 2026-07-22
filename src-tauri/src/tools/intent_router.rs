use tauri::AppHandle;

#[derive(Debug, Clone)]
pub struct AutoToolResult {
    pub tool_name: String,
    pub output: String,
}

pub fn auto_route_user_intent(app: &AppHandle, user_text: &str) -> Option<AutoToolResult> {
    let text = user_text.trim().to_lowercase();

    // 1. System status / VRAM / Hardware check
    if text.contains("vram")
        || text.contains("ram")
        || text.contains("สเป็ก")
        || text.contains("ฮาร์ดแวร์")
        || text.contains("gpu")
        || text.contains("system status")
        || text.contains("hardware")
    {
        if let Ok(output) = super::system::get_system_status(app) {
            return Some(AutoToolResult {
                tool_name: "get_system_status".to_string(),
                output,
            });
        }
    }

    // 2. Chat history / past sessions / personal identity lookup
    if text.contains("เมื่อวาน")
        || text.contains("ประวัติ")
        || text.contains("แชตเก่า")
        || text.contains("เคยคุย")
        || text.contains("past chat")
        || text.contains("chat history")
        || text.contains("previous session")
        || text.contains("ผมคือใคร")
        || text.contains("ผมชื่ออะไร")
        || text.contains("ฉันคือใคร")
        || text.contains("ฉันชื่ออะไร")
        || text.contains("จำผมได้ไหม")
        || text.contains("who am i")
        || text.contains("what is my name")
    {
        let query_term = if text.contains("ชื่อ") || text.contains("name") || text.contains("ใคร") || text.contains("who") {
            "ชื่อ"
        } else {
            "ผม"
        };

        if let Ok(output) = super::history::search_chat_history(app, query_term, 5) {
            return Some(AutoToolResult {
                tool_name: "search_chat_history".to_string(),
                output,
            });
        }
    }

    // 3. Installed models check
    if text.contains("โมเดลที่มี")
        || text.contains("โมเดลในเครื่อง")
        || text.contains("installed models")
        || text.contains("my models")
    {
        if let Ok(output) = super::models::list_installed_models(app) {
            return Some(AutoToolResult {
                tool_name: "list_installed_models".to_string(),
                output,
            });
        }
    }

    // 4. Workspace files list / check
    if text.contains("ไฟล์ในโปรเจกต์")
        || text.contains("ไฟล์งาน")
        || text.contains("workspace files")
        || text.contains("list files")
    {
        if let Ok(output) = super::workspace::list_workspace_files(app, None) {
            return Some(AutoToolResult {
                tool_name: "list_workspace_files".to_string(),
                output,
            });
        }
    }

    // 5. Explicit math calculation (contains math symbols and numbers)
    if (text.starts_with("คำนวณ") || text.starts_with("calc") || text.starts_with("calculate") || text.starts_with("eval"))
        && text.chars().any(|c| c.is_ascii_digit())
    {
        let expr = text
            .replace("คำนวณ", "")
            .replace("calc", "")
            .replace("calculate", "")
            .replace("eval", "")
            .replace("เท่าไหร่", "")
            .replace("เท่าไร", "")
            .replace("?", "")
            .trim()
            .to_string();

        if let Ok(output) = super::evaluator::evaluate_expression(&expr) {
            return Some(AutoToolResult {
                tool_name: "evaluate_expression".to_string(),
                output,
            });
        }
    }

    None
}
