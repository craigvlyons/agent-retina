use crate::Embedder;
use chrono::{DateTime, Utc};
use retina_types::*;
use rusqlite::{Connection, params};
use serde_json::json;
use sqlite_vec::sqlite3_vec_init;
use std::sync::Once;

static REGISTER_VEC: Once = Once::new();

pub(crate) fn register_sqlite_vec() {
    REGISTER_VEC.call_once(|| unsafe {
        let init: unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut i8,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> i32 = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut i8,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> i32,
        >(sqlite3_vec_init as *const ());
        rusqlite::ffi::sqlite3_auto_extension(Some(init));
    });
}

pub(crate) fn to_storage(error: impl ToString) -> KernelError {
    KernelError::Storage(error.to_string())
}

pub(crate) fn count_table(conn: &Connection, table: &str) -> Result<usize> {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get::<_, i64>(0)
    })
    .map(|value| value as usize)
    .map_err(to_storage)
}

pub(crate) fn parse_datetime(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

pub(crate) fn embedding_json(values: &[f32]) -> String {
    let output = values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("[{output}]")
}

pub(crate) fn sanitize_fts_query(query: &str) -> Option<String> {
    let tokens = query
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token.replace('"', "")))
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

pub(crate) fn persist_knowledge(
    conn: &Connection,
    embedder: &Embedder,
    node: &KnowledgeNode,
) -> Result<KnowledgeId> {
    let id = node.id.clone().unwrap_or_default();
    conn.execute(
        "INSERT OR REPLACE INTO knowledge (id, category, content, confidence, created_at, updated_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id.0,
            node.category,
            node.content,
            node.confidence,
            node.created_at.to_rfc3339(),
            node.updated_at.to_rfc3339(),
            serde_json::to_string(&node.metadata).map_err(to_storage)?,
        ],
    )
    .map_err(to_storage)?;
    conn.execute(
        "INSERT OR IGNORE INTO knowledge_id_map (knowledge_id) VALUES (?1)",
        params![id.0],
    )
    .map_err(to_storage)?;
    let rowid: i64 = conn
        .query_row(
            "SELECT rowid FROM knowledge_id_map WHERE knowledge_id = ?1",
            params![id.0],
            |row| row.get(0),
        )
        .map_err(to_storage)?;
    let embedding = embedder.embed(&node.content);
    let vector_json = embedding_json(&embedding);
    let _ = conn.execute(
        "INSERT OR REPLACE INTO knowledge_vec (rowid, embedding) VALUES (?1, vec_f32(?2))",
        params![rowid, vector_json],
    );
    Ok(id)
}

pub(crate) fn persist_rule(conn: &Connection, rule: &ReflexiveRule) -> Result<RuleId> {
    let id = rule.id.clone().unwrap_or_default();
    conn.execute(
        "INSERT OR REPLACE INTO reflexive_rules (id, name, condition_json, action_json, confidence, active, last_fired)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id.0,
            rule.name,
            serde_json::to_string(&rule.condition).map_err(to_storage)?,
            serde_json::to_string(&rule.action).map_err(to_storage)?,
            rule.confidence,
            if rule.active { 1 } else { 0 },
            rule.last_fired.map(|value| value.to_rfc3339()),
        ],
    )
    .map_err(to_storage)?;
    Ok(id)
}

pub(crate) fn load_recent_experiences(conn: &Connection, limit: usize) -> Result<Vec<Experience>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, task_id, intent_id, action_summary, outcome, utility, created_at, metadata
             FROM experiences
             ORDER BY created_at DESC
             LIMIT ?1",
        )
        .map_err(to_storage)?;
    let rows = stmt
        .query_map(params![limit as i64], row_to_experience)
        .map_err(to_storage)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(to_storage)
}

pub(crate) fn load_knowledge_nodes(conn: &Connection) -> Result<Vec<KnowledgeNode>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, category, content, confidence, created_at, updated_at, metadata
             FROM knowledge",
        )
        .map_err(to_storage)?;
    let rows = stmt.query_map([], row_to_knowledge).map_err(to_storage)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(to_storage)
}

pub(crate) fn load_rules(conn: &Connection, active_only: bool) -> Result<Vec<ReflexiveRule>> {
    let sql = if active_only {
        "SELECT id, name, condition_json, action_json, confidence, active, last_fired
         FROM reflexive_rules WHERE active = 1"
    } else {
        "SELECT id, name, condition_json, action_json, confidence, active, last_fired
         FROM reflexive_rules"
    };
    let mut stmt = conn.prepare(sql).map_err(to_storage)?;
    let rows = stmt.query_map([], row_to_rule).map_err(to_storage)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(to_storage)
}

pub(crate) fn row_to_experience(row: &rusqlite::Row<'_>) -> rusqlite::Result<Experience> {
    let metadata: String = row.get(8)?;
    let created_at: String = row.get(7)?;
    Ok(Experience {
        id: Some(ExperienceId(row.get(0)?)),
        session_id: SessionId(row.get(1)?),
        task_id: TaskId(row.get(2)?),
        intent_id: IntentId(row.get(3)?),
        action_summary: row.get(4)?,
        outcome: row.get(5)?,
        utility: row.get(6)?,
        created_at: parse_datetime(&created_at),
        metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})),
    })
}

pub(crate) fn row_to_knowledge(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeNode> {
    let metadata: String = row.get(6)?;
    let created_at: String = row.get(4)?;
    let updated_at: String = row.get(5)?;
    Ok(KnowledgeNode {
        id: Some(KnowledgeId(row.get(0)?)),
        category: row.get(1)?,
        content: row.get(2)?,
        confidence: row.get(3)?,
        created_at: parse_datetime(&created_at),
        updated_at: parse_datetime(&updated_at),
        metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})),
    })
}

pub(crate) fn row_to_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReflexiveRule> {
    let condition_json: String = row.get(2)?;
    let action_json: String = row.get(3)?;
    let last_fired: Option<String> = row.get(6)?;
    Ok(ReflexiveRule {
        id: Some(RuleId(row.get(0)?)),
        name: row.get(1)?,
        condition: serde_json::from_str(&condition_json).unwrap_or(RuleCondition::Always),
        action: serde_json::from_str(&action_json)
            .unwrap_or(RuleAction::AddNote("invalid".to_string())),
        confidence: row.get(4)?,
        active: row.get::<_, i64>(5)? == 1,
        last_fired: last_fired.as_deref().map(parse_datetime),
    })
}

pub(crate) fn row_to_timeline_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimelineEvent> {
    let payload_json: String = row.get(12)?;
    let event_type_json: String = row.get(5)?;
    let timestamp: String = row.get(4)?;
    Ok(TimelineEvent {
        event_id: EventId(row.get(0)?),
        session_id: SessionId(row.get(1)?),
        task_id: TaskId(row.get(2)?),
        agent_id: AgentId(row.get(3)?),
        timestamp: parse_datetime(&timestamp),
        event_type: serde_json::from_str(&event_type_json).unwrap_or(TimelineEventType::TaskFailed),
        intent_id: row.get::<_, Option<String>>(6)?.map(IntentId),
        action_id: row.get::<_, Option<String>>(7)?.map(ActionId),
        pre_state_hash: row.get(8)?,
        post_state_hash: row.get(9)?,
        delta_summary: row.get(10)?,
        duration_ms: row.get::<_, Option<i64>>(11)?.map(|value| value as u64),
        payload_json: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
    })
}
