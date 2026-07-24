use super::types::{InteractionOption, PendingInteraction, SessionDetail, SessionMessage, SessionSummary};
use crate::web_search::{RetrievalTraceEntry, SearchResult, WebSource};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn utc_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn database_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not resolve app data directory: {error}"))?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create app data directory: {error}"))?;
    Ok(directory.join("harness.db"))
}

fn open(app: &AppHandle) -> Result<Connection, String> {
    let connection = Connection::open(database_path(app)?)
        .map_err(|error| format!("Could not open local chat database: {error}"))?;
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
            retrieval_trace TEXT,
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
         CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);
         CREATE TABLE IF NOT EXISTS active_constraints (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            text TEXT NOT NULL,
            scope TEXT NOT NULL,
            created_at TEXT NOT NULL,
            superseded_by TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_constraints_session ON active_constraints(session_id);
         CREATE TABLE IF NOT EXISTS long_term_facts (
            id TEXT PRIMARY KEY,
            category TEXT NOT NULL,
            content TEXT NOT NULL,
            source_session_id TEXT REFERENCES sessions(id),
            confidence REAL NOT NULL,
            created_at TEXT NOT NULL,
            last_confirmed_at TEXT NOT NULL,
            superseded_by TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_lt_facts_category ON long_term_facts(category);
         CREATE TABLE IF NOT EXISTS session_summaries (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            summary TEXT NOT NULL,
            updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS pending_interactions (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            request_content TEXT NOT NULL,
            question TEXT NOT NULL,
            options_json TEXT NOT NULL,
            reason TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'pending',
            selected_option_id TEXT,
            created_at TEXT NOT NULL,
            resolved_at TEXT
         );",
    ).map_err(|error| format!("Could not migrate local chat database: {error}"))?;
    let mut columns = connection
        .prepare("PRAGMA table_info(messages)")
        .map_err(|error| format!("Could not inspect local chat schema: {error}"))?;
    let columns = columns
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Could not inspect local chat columns: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read local chat columns: {error}"))?;
    if !columns.iter().any(|column| column == "web_sources") {
        connection
            .execute("ALTER TABLE messages ADD COLUMN web_sources TEXT", [])
            .map_err(|error| format!("Could not extend local chat schema: {error}"))?;
    }
    if !columns.iter().any(|column| column == "retrieval_trace") {
        connection
            .execute("ALTER TABLE messages ADD COLUMN retrieval_trace TEXT", [])
            .map_err(|error| format!("Could not extend local chat schema: {error}"))?;
    }
    Ok(())
}

pub fn create_pending_interaction(
    app: &AppHandle,
    session_id: &str,
    request_content: &str,
    question: &str,
    option_labels: &[String],
    reason: &str,
) -> Result<PendingInteraction, String> {
    let interaction = PendingInteraction {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        request_content: request_content.to_string(),
        question: question.trim().to_string(),
        options: option_labels.iter().map(|label| InteractionOption {
            id: Uuid::new_v4().to_string(), label: label.trim().to_string(),
        }).collect(),
        reason: reason.trim().to_string(),
        created_at: utc_now(),
    };
    let connection = open(app)?;
    connection.execute(
        "UPDATE pending_interactions SET status = 'superseded', resolved_at = ?1 WHERE session_id = ?2 AND status = 'pending'",
        params![utc_now(), session_id],
    ).map_err(|error| format!("Could not supersede pending interaction: {error}"))?;
    connection.execute(
        "INSERT INTO pending_interactions (id, session_id, request_content, question, options_json, reason, status, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
        params![interaction.id, interaction.session_id, interaction.request_content, interaction.question, serde_json::to_string(&interaction.options).map_err(|error| format!("Could not serialize interaction options: {error}"))?, interaction.reason, interaction.created_at],
    ).map_err(|error| format!("Could not save pending interaction: {error}"))?;
    Ok(interaction)
}

pub fn resolve_pending_interaction(
    app: &AppHandle,
    interaction_id: &str,
    option_id: &str,
    session_id: &str,
) -> Result<(PendingInteraction, InteractionOption), String> {
    let connection = open(app)?;
    let interaction = connection.query_row(
        "SELECT id, session_id, request_content, question, options_json, reason, created_at FROM pending_interactions WHERE id = ?1 AND session_id = ?2 AND status = 'pending'",
        params![interaction_id, session_id],
        |row| {
            let options: Vec<InteractionOption> = serde_json::from_str(&row.get::<_, String>(4)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(PendingInteraction { id: row.get(0)?, session_id: row.get(1)?, request_content: row.get(2)?, question: row.get(3)?, options, reason: row.get(5)?, created_at: row.get(6)? })
        },
    ).optional().map_err(|error| format!("Could not load pending interaction: {error}"))?
      .ok_or_else(|| "This choice is no longer active. Please ask again.".to_string())?;
    let option = interaction
        .options
        .iter()
        .find(|option| option.id == option_id)
        .cloned()
        .unwrap_or_else(|| InteractionOption {
            id: option_id.to_string(),
            label: option_id.to_string(),
        });
    connection.execute(
        "UPDATE pending_interactions SET status = 'resolved', selected_option_id = ?1, resolved_at = ?2 WHERE id = ?3 AND status = 'pending'",
        params![option_id, utc_now(), interaction_id],
    ).map_err(|error| format!("Could not resolve pending interaction: {error}"))?;
    Ok((interaction, option))
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
        model_id: row.get(4)?,
    })
}

pub fn create(app: &AppHandle, model_id: Option<String>) -> Result<SessionSummary, String> {
    let session = SessionSummary {
        id: Uuid::new_v4().to_string(),
        title: "New chat".to_string(),
        created_at: now(),
        updated_at: now(),
        model_id,
    };
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
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| format!("Could not query chat sessions: {error}"))?;
    let rows = if query.is_empty() {
        statement.query_map([], summary_from_row)
    } else {
        statement.query_map([query], summary_from_row)
    }
    .map_err(|error| format!("Could not read chat sessions: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode chat sessions: {error}"))
}

pub fn get(app: &AppHandle, session_id: &str) -> Result<SessionDetail, String> {
    let connection = open(app)?;
    let (session, memory) = connection.query_row("SELECT id, title, created_at, updated_at, model_id, conversation_memory FROM sessions WHERE id = ?1", [session_id], |row| Ok((summary_from_row(row)?, row.get::<_, String>(5)?))).optional().map_err(|error| format!("Could not read chat session: {error}"))?.ok_or_else(|| "Chat session was not found.".to_string())?;
    let mut statement = connection.prepare("SELECT id, role, content, thinking_summary, finish_reason, web_sources, retrieval_trace, created_at, sequence FROM messages WHERE session_id = ?1 ORDER BY sequence ASC").map_err(|error| format!("Could not read chat messages: {error}"))?;
    let messages = statement
        .query_map([session_id], |row| {
            let web_sources = row
                .get::<_, Option<String>>(5)?
                .and_then(|value| serde_json::from_str(&value).ok());
            let retrieval_trace = row
                .get::<_, Option<String>>(6)?
                .and_then(|value| serde_json::from_str(&value).ok());
            Ok(SessionMessage {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                thinking_summary: row.get(3)?,
                finish_reason: row.get(4)?,
                web_sources,
                retrieval_trace,
                created_at: row.get(7)?,
                sequence: row.get(8)?,
            })
        })
        .map_err(|error| format!("Could not query chat messages: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode chat messages: {error}"))?;
    Ok(SessionDetail {
        session,
        messages,
        conversation_memory: memory,
    })
}

pub fn append_message(
    app: &AppHandle,
    session_id: &str,
    role: &str,
    content: &str,
    thinking_summary: Option<&str>,
    finish_reason: Option<&str>,
    web_sources: Option<&[WebSource]>,
    retrieval_trace: Option<&[RetrievalTraceEntry]>,
) -> Result<(), String> {
    let mut connection = open(app)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start chat write: {error}"))?;
    let sequence = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), -1) + 1 FROM messages WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Could not sequence chat message: {error}"))?;
    let timestamp = utc_now();
    let web_sources = web_sources
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| format!("Could not serialize web citations: {error}"))?;
    let retrieval_trace = retrieval_trace
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| format!("Could not serialize retrieval trace: {error}"))?;
    transaction.execute("INSERT INTO messages (id, session_id, role, content, thinking_summary, finish_reason, web_sources, retrieval_trace, created_at, sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)", params![Uuid::new_v4().to_string(), session_id, role, content, thinking_summary, finish_reason, web_sources, retrieval_trace, timestamp, sequence]).map_err(|error| format!("Could not save chat message: {error}"))?;
    if role == "user" {
        transaction.execute("UPDATE sessions SET title = CASE WHEN title = 'New chat' THEN ?1 ELSE title END, updated_at = ?2 WHERE id = ?3", params![fallback_title(content), now(), session_id]).map_err(|error| format!("Could not update chat session: {error}"))?;
    } else {
        transaction
            .execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                params![now(), session_id],
            )
            .map_err(|error| format!("Could not update chat session: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Could not finalize chat write: {error}"))
}

pub fn rename(app: &AppHandle, session_id: &str, title: &str) -> Result<SessionSummary, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("A chat title cannot be empty.".to_string());
    }
    let connection = open(app)?;
    connection
        .execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![truncate(title, 96), now(), session_id],
        )
        .map_err(|error| format!("Could not rename chat session: {error}"))?;
    get(app, session_id).map(|detail| detail.session)
}

pub fn delete(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let connection = open(app)?;
    connection
        .execute("DELETE FROM sessions WHERE id = ?1", [session_id])
        .map_err(|error| format!("Could not delete chat session: {error}"))?;
    Ok(())
}

pub fn set_memory(app: &AppHandle, session_id: &str, memory: &str) -> Result<(), String> {
    let connection = open(app)?;
    connection
        .execute(
            "UPDATE sessions SET conversation_memory = ?1 WHERE id = ?2",
            params![memory, session_id],
        )
        .map_err(|error| format!("Could not save conversation memory: {error}"))?;
    Ok(())
}

pub fn set_title(app: &AppHandle, session_id: &str, title: &str) -> Result<SessionSummary, String> {
    rename(app, session_id, title)
}

pub fn load_web_cache(app: &AppHandle, query: &str) -> Result<Vec<SearchResult>, String> {
    let connection = open(app)?;
    let cutoff = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().saturating_sub(3_600_000) as i64)
        .unwrap_or_default();
    connection
        .execute("DELETE FROM web_cache WHERE fetched_at < ?1", [cutoff])
        .map_err(|error| format!("Could not expire web cache: {error}"))?;
    let mut statement = connection.prepare("SELECT title, url, snippet, content FROM web_cache WHERE query = ?1 AND fetched_at >= ?2 ORDER BY fetched_at DESC LIMIT 5").map_err(|error| format!("Could not read web cache: {error}"))?;
    let rows = statement
        .query_map(params![query, cutoff], |row| {
            Ok(SearchResult {
                title: row.get(0)?,
                url: row.get(1)?,
                snippet: row.get(2)?,
                content: row.get(3)?,
            })
        })
        .map_err(|error| format!("Could not query web cache: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode web cache: {error}"))
}

pub fn save_web_cache(
    app: &AppHandle,
    query: &str,
    results: &[SearchResult],
) -> Result<(), String> {
    let mut connection = open(app)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not begin web-cache write: {error}"))?;
    let fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default();
    for result in results {
        transaction.execute(
            "INSERT INTO web_cache (query, url, title, snippet, content, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(query, url) DO UPDATE SET title = excluded.title, snippet = excluded.snippet, content = excluded.content, fetched_at = excluded.fetched_at",
            params![query, result.url, result.title, result.snippet, result.content, fetched_at],
        ).map_err(|error| format!("Could not cache web result: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Could not finalize web cache: {error}"))
}

// Tiered Memory DB Helpers

pub fn save_constraint(
    app: &AppHandle,
    session_id: &str,
    text: &str,
    scope: &str,
) -> Result<String, String> {
    let connection = open(app)?;
    let id = Uuid::new_v4().to_string();
    let created_at = utc_now();
    connection.execute(
        "INSERT INTO active_constraints (id, session_id, text, scope, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, session_id, text, scope, created_at],
    ).map_err(|e| format!("Could not save constraint: {e}"))?;
    Ok(id)
}

pub fn get_active_constraints(
    app: &AppHandle,
    session_id: &str,
) -> Result<Vec<(String, String, String, String)>, String> {
    let connection = open(app)?;
    let mut statement = connection.prepare(
        "SELECT id, text, scope, created_at FROM active_constraints WHERE session_id = ?1 AND superseded_by IS NULL ORDER BY created_at ASC",
    ).map_err(|e| format!("Could not prepare active constraints query: {e}"))?;
    let rows = statement
        .query_map([session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| format!("Could not query active constraints: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Could not decode constraints: {e}"))
}

#[allow(dead_code)]
pub fn supersede_constraint(app: &AppHandle, old_id: &str, new_id: &str) -> Result<(), String> {
    let connection = open(app)?;
    connection
        .execute(
            "UPDATE active_constraints SET superseded_by = ?1 WHERE id = ?2",
            params![new_id, old_id],
        )
        .map_err(|e| format!("Could not supersede constraint: {e}"))?;
    Ok(())
}

pub fn expire_turn_constraints(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let connection = open(app)?;
    connection
        .execute(
            "DELETE FROM active_constraints WHERE session_id = ?1 AND scope = 'turn_only'",
            params![session_id],
        )
        .map_err(|e| format!("Could not expire turn constraints: {e}"))?;
    Ok(())
}

#[allow(dead_code)]
pub fn save_long_term_fact(
    app: &AppHandle,
    category: &str,
    content: &str,
    source_session: Option<&str>,
    confidence: f32,
) -> Result<String, String> {
    let connection = open(app)?;
    let id = Uuid::new_v4().to_string();
    let created_at = utc_now();
    connection.execute(
        "INSERT INTO long_term_facts (id, category, content, source_session_id, confidence, created_at, last_confirmed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![id, category, content, source_session, confidence, created_at],
    ).map_err(|e| format!("Could not save long-term fact: {e}"))?;
    Ok(id)
}

pub fn get_all_long_term_facts(
    app: &AppHandle,
) -> Result<Vec<(String, String, String, Option<String>, f32, String)>, String> {
    let connection = open(app)?;
    let mut statement = connection.prepare(
        "SELECT id, category, content, source_session_id, confidence, last_confirmed_at FROM long_term_facts WHERE superseded_by IS NULL ORDER BY created_at DESC",
    ).map_err(|e| format!("Could not prepare long-term facts query: {e}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(|e| format!("Could not query long-term facts: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Could not decode long-term facts: {e}"))
}

#[allow(dead_code)]
pub fn supersede_long_term_fact(app: &AppHandle, old_id: &str, new_id: &str) -> Result<(), String> {
    let connection = open(app)?;
    connection
        .execute(
            "UPDATE long_term_facts SET superseded_by = ?1 WHERE id = ?2",
            params![new_id, old_id],
        )
        .map_err(|e| format!("Could not supersede long-term fact: {e}"))?;
    Ok(())
}

#[allow(dead_code)]
pub fn save_session_summary(
    app: &AppHandle,
    session_id: &str,
    summary: &str,
) -> Result<(), String> {
    let connection = open(app)?;
    let updated_at = utc_now();
    connection.execute(
        "INSERT INTO session_summaries (session_id, summary, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(session_id) DO UPDATE SET summary = excluded.summary, updated_at = excluded.updated_at",
        params![session_id, summary, updated_at],
    ).map_err(|e| format!("Could not save session summary: {e}"))?;
    Ok(())
}

#[allow(dead_code)]
pub fn get_all_session_summaries(app: &AppHandle) -> Result<Vec<(String, String)>, String> {
    let connection = open(app)?;
    let mut statement = connection
        .prepare("SELECT session_id, summary FROM session_summaries ORDER BY updated_at DESC")
        .map_err(|e| format!("Could not prepare session summaries query: {e}"))?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("Could not query session summaries: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Could not decode session summaries: {e}"))
}

fn fallback_title(value: &str) -> String {
    truncate(
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .as_str(),
        56,
    )
}

fn truncate(value: &str, max_chars: usize) -> String {
    let shortened = value.chars().take(max_chars).collect::<String>();
    if shortened.chars().count() < value.chars().count() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CrossSessionMatch {
    pub session_title: String,
    pub user_content: String,
    pub assistant_content: Option<String>,
    pub created_at: String,
}

pub fn search_cross_session_messages(
    app: &AppHandle,
    current_session_id: &str,
    query: &str,
    limit: usize,
) -> Vec<CrossSessionMatch> {
    let connection = match open(app) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let clean_query = query.trim();
    let tokens: Vec<String> = clean_query
        .split_whitespace()
        .filter(|t| t.chars().count() >= 2)
        .map(|t| t.to_lowercase())
        .collect();

    let sql = "SELECT m.content, m.created_at, s.title,
                (SELECT m2.content FROM messages m2 WHERE m2.session_id = m.session_id AND m2.sequence > m.sequence AND m2.role = 'assistant' ORDER BY m2.sequence ASC LIMIT 1) as assistant_reply
               FROM messages m
               JOIN sessions s ON s.id = m.session_id
               WHERE m.session_id != ?1 AND m.role = 'user'
               ORDER BY m.created_at DESC LIMIT 50";

    let mut stmt = match connection.prepare(sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map(params![current_session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    });

    let mut matches = Vec::new();
    if let Ok(iter) = rows {
        for item in iter.flatten() {
            let (user_text, created_at, session_title, assistant_reply) = item;
            let text_lower = user_text.to_lowercase();

            let score = if tokens.is_empty() {
                1
            } else {
                tokens.iter().filter(|t| text_lower.contains(*t)).count()
            };

            if score > 0 {
                matches.push((
                    score,
                    CrossSessionMatch {
                        session_title,
                        user_content: user_text,
                        assistant_content: assistant_reply,
                        created_at,
                    },
                ));
            }
        }
    }

    matches.sort_by(|a, b| b.0.cmp(&a.0));
    matches.into_iter().take(limit).map(|m| m.1).collect()
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

        connection
            .execute("DELETE FROM sessions WHERE id = ?1", ["session-1"])
            .expect("delete session");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .expect("count messages");
        assert_eq!(count, 0);

        let cache_table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'web_cache'",
                [],
                |row| row.get(0),
            )
            .expect("web cache table");
        assert_eq!(cache_table_count, 1);
        let columns = connection
            .prepare("PRAGMA table_info(messages)")
            .expect("message table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("message columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("read message columns");
        assert!(columns.iter().any(|column| column == "web_sources"));
        assert!(columns.iter().any(|column| column == "retrieval_trace"));
        let interaction_table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'pending_interactions'",
                [],
                |row| row.get(0),
            )
            .expect("interaction table");
        assert_eq!(interaction_table_count, 1);
    }

    #[test]
    fn migration_adds_trace_column_to_existing_messages() {
        let connection = Connection::open_in_memory().expect("in-memory database");
        connection
            .execute_batch(
                "CREATE TABLE messages (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    thinking_summary TEXT,
                    thinking_full TEXT,
                    finish_reason TEXT,
                    web_sources TEXT,
                    created_at TEXT NOT NULL,
                    sequence INTEGER NOT NULL
                );",
            )
            .expect("legacy message table");

        migrate(&connection).expect("migrate legacy schema");
        let columns = connection
            .prepare("PRAGMA table_info(messages)")
            .expect("message table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("message columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("read message columns");
        assert!(columns.iter().any(|column| column == "retrieval_trace"));
    }
}
