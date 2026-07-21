use super::types::{SessionDetail, SessionMessage, SessionSummary};
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
            created_at TEXT NOT NULL,
            sequence INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, sequence);
         CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at);
         CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);",
    ).map_err(|error| format!("Could not migrate local chat database: {error}"))
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
    let mut statement = connection.prepare("SELECT id, role, content, thinking_summary, finish_reason, created_at, sequence FROM messages WHERE session_id = ?1 ORDER BY sequence ASC").map_err(|error| format!("Could not read chat messages: {error}"))?;
    let messages = statement.query_map([session_id], |row| Ok(SessionMessage { id: row.get(0)?, role: row.get(1)?, content: row.get(2)?, thinking_summary: row.get(3)?, finish_reason: row.get(4)?, created_at: row.get(5)?, sequence: row.get(6)? })).map_err(|error| format!("Could not query chat messages: {error}"))?.collect::<Result<Vec<_>, _>>().map_err(|error| format!("Could not decode chat messages: {error}"))?;
    Ok(SessionDetail { session, messages, conversation_memory: memory })
}

pub fn append_message(app: &AppHandle, session_id: &str, role: &str, content: &str, thinking_summary: Option<&str>, finish_reason: Option<&str>) -> Result<(), String> {
    let mut connection = open(app)?;
    let transaction = connection.transaction().map_err(|error| format!("Could not start chat write: {error}"))?;
    let sequence = transaction.query_row("SELECT COALESCE(MAX(sequence), -1) + 1 FROM messages WHERE session_id = ?1", [session_id], |row| row.get::<_, i64>(0)).map_err(|error| format!("Could not sequence chat message: {error}"))?;
    let timestamp = utc_now();
    transaction.execute("INSERT INTO messages (id, session_id, role, content, thinking_summary, finish_reason, created_at, sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", params![Uuid::new_v4().to_string(), session_id, role, content, thinking_summary, finish_reason, timestamp, sequence]).map_err(|error| format!("Could not save chat message: {error}"))?;
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
    }
}
