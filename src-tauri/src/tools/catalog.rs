use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub tool_name: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub enum LoopStepResult {
    FinalAnswer {
        content: String,
        finish_reason: Option<String>,
    },
    ToolCalls(Vec<RequestedToolCall>),
}

pub fn tool_catalog() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "search_web".to_string(),
            description: "Search the web for real-time news, current events, or factual information not known to the model.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query string." }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "crawl_web_page".to_string(),
            description: "Deep-scrape and extract clean markdown content from a specific web page URL using Crawl4AI.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The target web page URL to crawl (e.g. https://news.ycombinator.com)." }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "get_weather".to_string(),
            description: "Get current weather conditions and forecasts for a specified city or location.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": { "type": "string", "description": "City or location name (e.g. Bangkok, Tokyo)." }
                },
                "required": ["location"]
            }),
        },
        ToolDefinition {
            name: "get_currency_rate".to_string(),
            description: "Get the current foreign exchange conversion rate between two currencies.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source currency code (e.g. USD, THB, EUR)." },
                    "to": { "type": "string", "description": "Target currency code (e.g. THB, JPY, USD)." }
                },
                "required": ["from", "to"]
            }),
        },
        ToolDefinition {
            name: "get_stock_price".to_string(),
            description: "Get current price and market metrics for a stock ticker or cryptocurrency.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ticker": { "type": "string", "description": "Stock or crypto symbol (e.g. AAPL, BTC, PTT.BK)." }
                },
                "required": ["ticker"]
            }),
        },
        ToolDefinition {
            name: "search_wikipedia".to_string(),
            description: "Search Wikipedia for definitional, encyclopedic, or historical background knowledge.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string", "description": "The topic or article term to look up." }
                },
                "required": ["topic"]
            }),
        },
        ToolDefinition {
            name: "get_system_status".to_string(),
            description: "Get system hardware status including CPU, GPU, VRAM usage, engine info, and local runtime release.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "list_models".to_string(),
            description: "List locally installed models or search HuggingFace GGUF models.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Optional search term for HuggingFace model search." }
                }
            }),
        },
        ToolDefinition {
            name: "list_workspace_files".to_string(),
            description: "Inspect and list files in the active project workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "subpath": { "type": "string", "description": "Optional relative subdirectory path." }
                }
            }),
        },
        ToolDefinition {
            name: "read_workspace_file".to_string(),
            description: "Read the contents of a specific file in the active project workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "relative_path": { "type": "string", "description": "Relative file path inside workspace." }
                },
                "required": ["relative_path"]
            }),
        },
        ToolDefinition {
            name: "evaluate_expression".to_string(),
            description: "Evaluate an explicit mathematical calculation or formula.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "Math expression (e.g. '32 * 1.8 + 32' or 'sqrt(144)')." }
                },
                "required": ["expression"]
            }),
        },
        ToolDefinition {
            name: "ask_user_clarification".to_string(),
            description: "Ask the user a clarifying question with 2-4 selectable options ONLY when the user's request genuinely lacks essential details, scope, or parameters needed to provide a good response.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The clarifying question to display on the UI." },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "2 to 4 distinct options for the user to choose from."
                    },
                    "reason": { "type": "string", "description": "Brief explanation of why clarification is needed." }
                },
                "required": ["question", "options"]
            }),
        },
        ToolDefinition {
            name: "search_chat_history".to_string(),
            description: "Search across past conversations in this session and previous sessions to find relevant context, earlier answers, or user-stated preferences. Use when the user refers to something mentioned before or asks what they discussed earlier.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search term or topic to look up in conversation history." }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "get_session_details".to_string(),
            description: "Retrieve the full message history of a specific past conversation session by its ID.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "The session UUID to retrieve." }
                },
                "required": ["session_id"]
            }),
        },
        ToolDefinition {
            name: "search_huggingface_models".to_string(),
            description: "Search HuggingFace for available GGUF AI models that can be downloaded and run locally. Use when the user asks about finding, comparing, or downloading a specific type of local AI model.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search term for the model type or name (e.g. 'llama 7b', 'mistral gguf', 'phi-3')." }
                },
                "required": ["query"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_catalog_contains_expected_tools() {
        let tools = tool_catalog();
        assert_eq!(tools.len(), 15);
        let names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"search_web".to_string()));
        assert!(names.contains(&"crawl_web_page".to_string()));
        assert!(names.contains(&"ask_user_clarification".to_string()));
        assert!(names.contains(&"get_weather".to_string()));
        assert!(names.contains(&"evaluate_expression".to_string()));
        assert!(names.contains(&"search_chat_history".to_string()));
        assert!(names.contains(&"get_session_details".to_string()));
        assert!(names.contains(&"search_huggingface_models".to_string()));
    }
}
