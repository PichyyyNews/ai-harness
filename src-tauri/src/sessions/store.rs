use super::types::{SessionDetail, SessionMessage, SessionSummary};
use crate::web_search::{SearchResult, WebSource};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::{path::PathBuf, time::{SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

fn now() -> String {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_millis().to_string()).unwrap_or_else(|_| "0".to_string())
}

fn utc_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn database_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app.path().app_data_dir().map_err(|error| format!("Could not resolve app data directory: {error}"))?;
    std::fs::create_dir_all(&directory).map_err(|error| format!("Could not create app data directory: {error}"))?;
    Ok(directory.join("harness.db"))
}

fn open(app: &AppHandle) -> Result<Connection, String> {
    let connection = Connection::open(database_path(app)?).map_err(|error| format!("Could not open local chat database: {error}"))?;
    migrate(&connection)?;
    Ok(connection)
}

fn migrate(connection: &Connection) -> Result<(), String> {
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
         INSERT INTO schema_version (version) SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version);
         CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            model_id TEXT,
            conversation_memory TEXT NOT NULL DEFAULT ''
         );
         CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            thinking_summary TEXT,
            thinking_full TEXT,
            finish_reason TEXT,
            web_sources TEXT,
            created_at TEXT NOT NULL,
            sequence INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, sequence);
         CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at);
         CREATE TABLE IF NOT EXISTS web_cache (
            query TEXT NOT NULL,
            url TEXT NOT NULL,
            title TEXT NOT NULL,
            snippet TEXT NOT NULL,
            content TEXT NOT NULL,
            fetched_at INTEGER NOT NULL,
            PRIMARY KEY (query, url)
         );
         CREATE INDEX IF NOT EXISTS idx_web_cache_query_time ON web_cache(query, fetched_at DESC);
         CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);",
    ).map_err(|error| format!("Could not migrate local chat database: {error}"))?;
    let mut columns = connection.prepare("PRAGMA table_info(messages)").map_err(|error| format!("Could not inspect local chat schema: {error}"))?;
    let has_web_sources = columns.query_map([], |row| row.get::<_, String>(1)).map_err(|error| format!("Could not inspect local chat columns: {error}"))?
        .collect::<Result<Vec<_>, _>>().map_err(|error| format!("Could not read local chat columns: {error}"))?
        .iter().any(|column| column == "web_sources");
    if !has_web_sources {
        connection.execute("ALTER TABLE messages ADD COLUMN web_sources TEXT", []).map_err(|error| format!("Could not extend local chat schema: {error}"))?;
    }
    Ok(())
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary { id: row.get(0)?, title: row.get(1)?, created_at: row.get(2)?, updated_at: row.get(3)?, model_id: row.get(4)? })
}

pub fn create(app: &AppHandle, model_id: Option<String>) -> Result<SessionSummary, String> {
    let session = SessionSummary { id: Uuid::new_v4().to_string(), title: "New chat".to_string(), created_at: now(), updated_at: now(), model_id };
    let connection = open(app)?;
    connection.execute("INSERT INTO sessions (id, title, created_at, updated_at, model_id, conversation_memory) VALUES (?1, ?2, ?3, ?4, ?5, '')", params![session.id, session.title, session.created_at, session.updated_at, session.model_id]).map_err(|error| format!("Could not create chat session: {error}"))?;
    Ok(session)
}

pub fn list(app: &AppHandle, query: Option<String>) -> Result<Vec<SessionSummary>, String> {
    let connection = open(app)?;
    let query = query.unwrap_or_default().trim().to_string();
    let sql = if query.is_empty() {
        "SELECT id, title, created_at, updated_at, model_id FROM sessions WHERE EXISTS (SELECT 1 FROM messages WHERE messages.session_id = sessions.id) ORDER BY updated_at DESC"
    } else {
        "SELECT id, title, created_at, updated_at, model_id FROM sessions WHERE EXISTS (SELECT 1 FROM messages WHERE messages.session_id = sessions.id) AND (title LIKE '%' || ?1 || '%' OR EXISTS (SELECT 1 FROM messages WHERE messages.session_id = sessions.id AND content LIKE '%' || ?1 || '%')) ORDER BY updated_at DESC"
    };
    let mut statement = connection.prepare(sql).map_err(|error| format!("Could not query chat sessions: {error}"))?;
    let rows = if query.is_empty() { statement.query_map([], summary_from_row) } else { statement.query_map([query], summary_from_row) }.map_err(|error| format!("Could not read chat sessions: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("Could not decode chat sessions: {error}"))
}

pub fn get(app: &AppHandle, session_id: &str) -> Result<SessionDetail, String> {
    let connection = open(app)?;
    let (session, memory) = connection.query_row("SELECT id, title, created_at, updated_at, model_id, conversation_memory FROM sessions WHERE id = ?1", [session_id], |row| Ok((summary_from_row(row)?, row.get::<_, String>(5)?))).optional().map_err(|error| format!("Could not read chat session: {error}"))?.ok_or_else(|| "Chat session was not found.".to_string())?;
    let mut statement = connection.prepare("SELECT id, role, content, thinking_summary, finish_reason, web_sources, created_at, sequence FROM messages WHERE session_id = ?1 ORDER BY sequence ASC").map_err(|error| format!("Could not read chat messages: {error}"))?;
    let messages = statement.query_map([session_id], |row| {
        let web_sources = row.get::<_, Option<String>>(5)?.and_then(|value| serde_json::from_str(&value).ok());
        Ok(SessionMessage { id: row.get(0)?, role: row.get(1)?, content: row.get(2)?, thinking_summary: row.get(3)?, finish_reason: row.get(4)?, web_sources, created_at: row.get(6)?, sequence: row.get(7)? })
    }).map_err(|error| format!("Could not query chat messages: {error}"))?.collect::<Result<Vec<_>, _>>().map_err(|error| format!("Could not decode chat messages: {error}"))?;
    Ok(SessionDetail { session, messages, conversation_memory: memory })
}

pub fn append_message(app: &AppHandle, session_id: &str, role: &str, content: &str, thinking_summary: Option<&str>, finish_reason: Option<&str>, web_sources: Option<&[WebSource]>) -> Result<(), String> {
    let mut connection = open(app)?;
    let transaction = connection.transaction().map_err(|error| format!("Could not start chat write: {error}"))?;
    let sequence = transaction.query_row("SELECT COALESCE(MAX(sequence), -1) + 1 FROM messages WHERE session_id = ?1", [session_id], |row| row.get::<_, i64>(0)).map_err(|error| format!("Could not sequence chat message: {error}"))?;
    let timestamp = utc_now();
    let web_sources = web_sources.map(serde_json::to_string).transpose().map_err(|error| format!("Could not serialize web citations: {error}"))?;
    transaction.execute("INSERT INTO messages (id, session_id, role, content, thinking_summary, finish_reason, web_sources, created_at, sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)", params![Uuid::new_v4().to_string(), session_id, role, content, thinking_summary, finish_reason, web_sources, timestamp, sequence]).map_err(|error| format!("Could not save chat message: {error}"))?;
    if role == "user" {
        transaction.execute("UPDATE sessions SET title = CASE WHEN title = 'New chat' THEN ?1 ELSE title END, updated_at = ?2 WHERE id = ?3", params![fallback_title(content), now(), session_id]).map_err(|error| format!("Could not update chat session: {error}"))?;
    } else {
        transaction.execute("UPDATE sessions SET updated_at = ?1 WHERE id = ?2", params![now(), session_id]).map_err(|error| format!("Could not update chat session: {error}"))?;
    }
    transaction.commit().map_err(|error| format!("Could not finalize chat write: {error}"))
}

pub fn rename(app: &AppHandle, session_id: &str, title: &str) -> Result<SessionSummary, String> {
    let title = title.trim();
    if title.is_empty() { return Err("A chat title cannot be empty.".to_string()); }
    let connection = open(app)?;
    connection.execute("UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3", params![truncate(title, 96), now(), session_id]).map_err(|error| format!("Could not rename chat session: {error}"))?;
    get(app, session_id).map(|detail| detail.session)
}

pub fn delete(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let connection = open(app)?;
    connection.execute("DELETE FROM sessions WHERE id = ?1", [session_id]).map_err(|error| format!("Could not delete chat session: {error}"))?;
    Ok(())
}

pub fn set_memory(app: &AppHandle, session_id: &str, memory: &str) -> Result<(), String> {
    let connection = open(app)?;
    connection.execute("UPDATE sessions SET conversation_memory = ?1 WHERE id = ?2", params![memory, session_id]).map_err(|error| format!("Could not save conversation memory: {error}"))?;
    Ok(())
}

pub fn set_title(app: &AppHandle, session_id: &str, title: &str) -> Result<SessionSummary, String> { rename(app, session_id, title) }

pub fn load_web_cache(app: &AppHandle, query: &str) -> Result<Vec<SearchResult>, String> {
    let connection = open(app)?;
    let cutoff = SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_millis().saturating_sub(3_600_000) as i64).unwrap_or_default();
    connection.execute("DELETE FROM web_cache WHERE fetched_at < ?1", [cutoff]).map_err(|error| format!("Could not expire web cache: {error}"))?;
    let mut statement = connection.prepare("SELECT title, url, snippet, content FROM web_cache WHERE query = ?1 AND fetched_at >= ?2 ORDER BY fetched_at DESC LIMIT 5").map_err(|error| format!("Could not read web cache: {error}"))?;
    let rows = statement.query_map(params![query, cutoff], |row| Ok(SearchResult {
        title: row.get(0)?, url: row.get(1)?, snippet: row.get(2)?, content: row.get(3)?,
    })).map_err(|error| format!("Could not query web cache: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("Could not decode web cache: {error}"))
}

pub fn save_web_cache(app: &AppHandle, query: &str, results: &[SearchResult]) -> Result<(), String> {
    let mut connection = open(app)?;
    let transaction = connection.transaction().map_err(|error| format!("Could not begin web-cache write: {error}"))?;
    let fetched_at = SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_millis() as i64).unwrap_or_default();
    for result in results {
        transaction.execute(
            "INSERT INTO web_cache (query, url, title, snippet, content, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(query, url) DO UPDATE SET title = excluded.title, snippet = excluded.snippet, content = excluded.content, fetched_at = excluded.fetched_at",
            params![query, result.url, result.title, result.snippet, result.content, fetched_at],
        ).map_err(|error| format!("Could not cache web result: {error}"))?;
    }
    transaction.commit().map_err(|error| format!("Could not finalize web cache: {error}"))
}

fn fallback_title(value: &str) -> String { truncate(value.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), 56) }

fn truncate(value: &str, max_chars: usize) -> String {
    let shortened = value.chars().take(max_chars).collect::<String>();
    if shortened.chars().count() < value.chars().count() { format!("{shortened}…") } else { shortened }
}

#[cfg(test)]
mod tests {
    use super::migrate;
    use rusqlite::{params, Connection};

    #[test]
    fn migration_creates_a_cascading_session_store() {
        let connection = Connection::open_in_memory().expect("in-memory database");
        migrate(&connection).expect("schema migration");

        connection.execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, conversation_memory) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["session-1", "A saved chat", "1", "1", "summary"],
        ).expect("insert session");
        connection.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["message-1", "session-1", "user", "hello", "1", 0],
        ).expect("insert message");

        connection.execute("DELETE FROM sessions WHERE id = ?1", ["session-1"]).expect("delete session");
        let count: i64 = connection.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0)).expect("count messages");
        assert_eq!(count, 0);

        let cache_table_count: i64 = connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'web_cache'", [], |row| row.get(0)).expect("web cache table");
        assert_eq!(cache_table_count, 1);
        let columns = connection.prepare("PRAGMA table_info(messages)").expect("message table info")
            .query_map([], |row| row.get::<_, String>(1)).expect("message columns")
            .collect::<Result<Vec<_>, _>>().expect("read message columns");
        assert!(columns.iter().any(|column| column == "web_sources"));
    }
}
