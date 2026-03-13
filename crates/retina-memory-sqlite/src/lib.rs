use chrono::{DateTime, Utc};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use refinery::embed_migrations;
use retina_traits::Memory;
use retina_types::*;
use rusqlite::{Connection, DatabaseName, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlite_vec::sqlite3_vec_init;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

embed_migrations!("migrations");

static REGISTER_VEC: Once = Once::new();

pub struct SqliteMemory {
    conn: Mutex<Connection>,
    embedder: Embedder,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStats {
    pub timeline_events: usize,
    pub experiences: usize,
    pub knowledge: usize,
    pub rules: usize,
    pub tools: usize,
}

impl SqliteMemory {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        register_sqlite_vec();
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn =
            Connection::open(path).map_err(|error| KernelError::Storage(error.to_string()))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA busy_timeout = 5000;
            PRAGMA foreign_keys = ON;
            ",
        )
        .map_err(|error| KernelError::Storage(error.to_string()))?;
        migrations::runner()
            .run(&mut conn)
            .map_err(|error| KernelError::Storage(error.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
            embedder: Embedder::new(),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory()
            .map_err(|error| KernelError::Storage(error.to_string()))?;
        migrations::runner()
            .run(&mut conn)
            .map_err(|error| KernelError::Storage(error.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
            embedder: Embedder::new(),
        })
    }

    fn with_conn<T>(&self, func: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().unwrap();
        func(&conn)
    }

    fn embed_text(&self, input: &str) -> Vec<f32> {
        self.embedder.embed(input)
    }

    pub fn save_manifest(&self, manifest: &AgentManifest) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO agent_manifest (agent_id, domain, status, description, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    manifest.agent_id.0,
                    manifest.domain,
                    serde_json::to_string(&manifest.status).map_err(to_storage)?,
                    manifest.description,
                    manifest.created_at.to_rfc3339(),
                ],
            )
            .map_err(to_storage)?;
            Ok(())
        })
    }

    pub fn load_manifest(&self, agent_id: &AgentId) -> Result<Option<AgentManifest>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT domain, status, description, created_at FROM agent_manifest WHERE agent_id = ?1",
                params![agent_id.0],
                |row| {
                    let status_json: String = row.get(1)?;
                    let created_at: String = row.get(3)?;
                    Ok(AgentManifest {
                        agent_id: agent_id.clone(),
                        domain: row.get(0)?,
                        status: serde_json::from_str(&status_json).unwrap_or(AgentStatus::Spawned),
                        description: row.get(2)?,
                        created_at: parse_datetime(&created_at),
                    })
                },
            )
            .optional()
            .map_err(to_storage)
        })
    }

    pub fn stats(&self) -> Result<MemoryStats> {
        self.with_conn(|conn| {
            Ok(MemoryStats {
                timeline_events: count_table(conn, "timeline_events")?,
                experiences: count_table(conn, "experiences")?,
                knowledge: count_table(conn, "knowledge")?,
                rules: count_table(conn, "reflexive_rules")?,
                tools: count_table(conn, "tool_registry")?,
            })
        })
    }
}

impl Memory for SqliteMemory {
    fn append_timeline_event(&self, event: &TimelineEvent) -> Result<()> {
        self.with_conn(|conn| {
            let payload = serde_json::to_string(&event.payload_json).map_err(to_storage)?;
            conn.execute(
                "INSERT INTO timeline_events
                (event_id, session_id, task_id, agent_id, timestamp, event_type, intent_id, action_id, pre_state_hash, post_state_hash, delta_summary, duration_ms, payload_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    event.event_id.0,
                    event.session_id.0,
                    event.task_id.0,
                    event.agent_id.0,
                    event.timestamp.to_rfc3339(),
                    serde_json::to_string(&event.event_type).map_err(to_storage)?,
                    event.intent_id.as_ref().map(|value| value.0.clone()),
                    event.action_id.as_ref().map(|value| value.0.clone()),
                    event.pre_state_hash,
                    event.post_state_hash,
                    event.delta_summary,
                    event.duration_ms.map(|value| value as i64),
                    payload,
                ],
            )
            .map_err(to_storage)?;
            conn.execute(
                "INSERT INTO state_log (event_id, session_id, task_id, timestamp, payload_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![event.event_id.0, event.session_id.0, event.task_id.0, event.timestamp.to_rfc3339(), serde_json::to_string(event).map_err(to_storage)?],
            )
            .map_err(to_storage)?;
            Ok(())
        })
    }

    fn record_experience(&self, exp: &Experience) -> Result<ExperienceId> {
        let id = exp.id.clone().unwrap_or_default();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO experiences
                (id, session_id, task_id, intent_id, action_summary, outcome, utility, created_at, metadata)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id.0,
                    exp.session_id.0,
                    exp.task_id.0,
                    exp.intent_id.0,
                    exp.action_summary,
                    exp.outcome,
                    exp.utility,
                    exp.created_at.to_rfc3339(),
                    serde_json::to_string(&exp.metadata).map_err(to_storage)?,
                ],
            )
            .map_err(to_storage)?;
            Ok(id)
        })
    }

    fn store_knowledge(&self, node: &KnowledgeNode) -> Result<KnowledgeId> {
        let id = node.id.clone().unwrap_or_default();
        self.with_conn(|conn| {
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
            let embedding = self.embed_text(&node.content);
            let vector_json = embedding_json(&embedding);
            let _ = conn.execute(
                "INSERT OR REPLACE INTO knowledge_vec (rowid, embedding) VALUES (?1, vec_f32(?2))",
                params![rowid, vector_json],
            );
            Ok(id)
        })
    }

    fn link_knowledge(&self, from: KnowledgeId, to: KnowledgeId, relation: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO knowledge_edges (source_id, target_id, relation) VALUES (?1, ?2, ?3)",
                params![from.0, to.0, relation],
            )
            .map_err(to_storage)?;
            Ok(())
        })
    }

    fn store_rule(&self, rule: &ReflexiveRule) -> Result<RuleId> {
        let id = rule.id.clone().unwrap_or_default();
        self.with_conn(|conn| {
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
        })
    }

    fn register_tool(&self, tool: &ToolRecord) -> Result<ToolId> {
        let id = tool.id.clone().unwrap_or_default();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO tool_registry (id, name, description, source_lang, test_status, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id.0,
                    tool.name,
                    tool.description,
                    serde_json::to_string(&tool.source_lang).map_err(to_storage)?,
                    tool.test_status,
                    serde_json::to_string(&tool.metadata).map_err(to_storage)?,
                ],
            )
            .map_err(to_storage)?;
            Ok(id)
        })
    }

    fn append_state(&self, entry: &TimelineEvent) -> Result<()> {
        self.append_timeline_event(entry)
    }

    fn recall_experiences(&self, query: &str, limit: usize) -> Result<Vec<Experience>> {
        self.with_conn(|conn| {
            let pattern = format!("%{}%", query);
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, task_id, intent_id, action_summary, outcome, utility, created_at, metadata
                     FROM experiences
                     WHERE action_summary LIKE ?1 OR outcome LIKE ?1 OR metadata LIKE ?1
                     ORDER BY utility DESC, created_at DESC
                     LIMIT ?2",
                )
                .map_err(to_storage)?;
            let rows = stmt
                .query_map(params![pattern, limit as i64], row_to_experience)
                .map_err(to_storage)?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(to_storage)
        })
    }

    fn recall_knowledge(&self, query: &str, limit: usize) -> Result<Vec<KnowledgeNode>> {
        self.with_conn(|conn| {
            let mut scores: HashMap<String, f64> = HashMap::new();
            let mut matched_ids = HashSet::new();
            let fts_query = sanitize_fts_query(query);

            if let Some(fts_query) = fts_query {
                let mut fts_stmt = conn
                    .prepare(
                        "SELECT knowledge_id FROM knowledge_id_map
                         WHERE rowid IN (
                            SELECT rowid FROM knowledge_fts
                            WHERE knowledge_fts MATCH ?1
                            LIMIT ?2
                         )",
                    )
                    .map_err(to_storage)?;
                let fts_ids = fts_stmt
                    .query_map(params![fts_query, limit as i64], |row| row.get::<_, String>(0))
                    .map_err(to_storage)?;
                for (rank, item) in fts_ids.enumerate() {
                    let id = item.map_err(to_storage)?;
                    matched_ids.insert(id.clone());
                    scores.insert(id, 1.0 / (rank as f64 + 1.0));
                }
            }

            let embedding = self.embed_text(query);
            let vector_json = embedding_json(&embedding);
            if let Ok(mut vec_stmt) = conn.prepare(
                "SELECT knowledge_id_map.knowledge_id, knowledge_vec.distance
                 FROM knowledge_vec
                 JOIN knowledge_id_map ON knowledge_id_map.rowid = knowledge_vec.rowid
                 WHERE knowledge_vec.embedding MATCH vec_f32(?1)
                   AND k = ?2
                 ORDER BY knowledge_vec.distance",
            ) {
                let rows = vec_stmt.query_map(params![vector_json, limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
                });
                if let Ok(rows) = rows {
                    for (rank, item) in rows.enumerate() {
                        let (id, distance) = item.map_err(to_storage)?;
                        matched_ids.insert(id.clone());
                        let current = scores.entry(id).or_default();
                        *current += 1.0 / (rank as f64 + 1.0) + (1.0 / (1.0 + distance));
                    }
                }
            }

            if matched_ids.is_empty() {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, category, content, confidence, created_at, updated_at, metadata
                         FROM knowledge
                         ORDER BY updated_at DESC
                         LIMIT ?1",
                    )
                    .map_err(to_storage)?;
                let rows = stmt
                    .query_map(params![limit as i64], row_to_knowledge)
                    .map_err(to_storage)?;
                return rows.collect::<std::result::Result<Vec<_>, _>>().map_err(to_storage);
            }

            let mut ranked = matched_ids.into_iter().collect::<Vec<_>>();
            ranked.sort_by(|left, right| {
                scores
                    .get(right)
                    .partial_cmp(&scores.get(left))
                    .unwrap_or(Ordering::Equal)
            });
            ranked.truncate(limit);

            let mut output = Vec::new();
            for knowledge_id in ranked {
                let node = conn
                    .query_row(
                        "SELECT id, category, content, confidence, created_at, updated_at, metadata FROM knowledge WHERE id = ?1",
                        params![knowledge_id],
                        row_to_knowledge,
                    )
                    .map_err(to_storage)?;
                output.push(node);
            }
            Ok(output)
        })
    }

    fn active_rules(&self) -> Result<Vec<ReflexiveRule>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, condition_json, action_json, confidence, active, last_fired
                     FROM reflexive_rules WHERE active = 1",
                )
                .map_err(to_storage)?;
            let rows = stmt
                .query_map([], |row| {
                    let condition_json: String = row.get(2)?;
                    let action_json: String = row.get(3)?;
                    let last_fired: Option<String> = row.get(6)?;
                    Ok(ReflexiveRule {
                        id: Some(RuleId(row.get::<_, String>(0)?)),
                        name: row.get(1)?,
                        condition: serde_json::from_str(&condition_json)
                            .unwrap_or(RuleCondition::Always),
                        action: serde_json::from_str(&action_json)
                            .unwrap_or(RuleAction::AddNote("invalid".to_string())),
                        confidence: row.get(4)?,
                        active: row.get::<_, i64>(5)? == 1,
                        last_fired: last_fired.as_deref().map(parse_datetime),
                    })
                })
                .map_err(to_storage)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(to_storage)
        })
    }

    fn find_tools(&self, capability: &str) -> Result<Vec<ToolRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, description, source_lang, test_status, metadata
                     FROM tool_registry
                     WHERE description LIKE ?1 OR name LIKE ?1
                     ORDER BY name ASC",
                )
                .map_err(to_storage)?;
            let pattern = format!("%{}%", capability);
            let rows = stmt
                .query_map(params![pattern], |row| {
                    let source_lang_json: String = row.get(3)?;
                    let metadata_json: String = row.get(5)?;
                    Ok(ToolRecord {
                        id: Some(ToolId(row.get::<_, String>(0)?)),
                        name: row.get(1)?,
                        description: row.get(2)?,
                        source_lang: serde_json::from_str(&source_lang_json)
                            .unwrap_or(SourceLanguage::Other("unknown".to_string())),
                        test_status: row.get(4)?,
                        metadata: serde_json::from_str(&metadata_json)
                            .unwrap_or_else(|_| json!({})),
                    })
                })
                .map_err(to_storage)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(to_storage)
        })
    }

    fn recent_states(&self, limit: usize) -> Result<Vec<TimelineEvent>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT event_id, session_id, task_id, agent_id, timestamp, event_type, intent_id, action_id, pre_state_hash, post_state_hash, delta_summary, duration_ms, payload_json
                     FROM timeline_events
                     ORDER BY timestamp DESC
                     LIMIT ?1",
                )
                .map_err(to_storage)?;
            let rows = stmt
                .query_map(params![limit as i64], row_to_timeline_event)
                .map_err(to_storage)?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(to_storage)
        })
    }

    fn update_utility(&self, id: ExperienceId, utility: f64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE experiences SET utility = ?2 WHERE id = ?1",
                params![id.0, utility],
            )
            .map_err(to_storage)?;
            Ok(())
        })
    }

    fn update_knowledge(&self, id: KnowledgeId, update: &KnowledgeUpdate) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE knowledge SET confidence = COALESCE(?2, confidence), metadata = COALESCE(?3, metadata), updated_at = ?4 WHERE id = ?1",
                params![
                    id.0,
                    update.confidence,
                    update.metadata.as_ref().map(|value| serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())),
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(to_storage)?;
            Ok(())
        })
    }

    fn update_rule(&self, id: RuleId, update: &RuleUpdate) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE reflexive_rules
                 SET confidence = COALESCE(?2, confidence),
                     active = COALESCE(?3, active),
                     last_fired = COALESCE(?4, last_fired)
                 WHERE id = ?1",
                params![
                    id.0,
                    update.confidence,
                    update.active.map(|value| if value { 1 } else { 0 }),
                    update.last_fired.map(|value| value.to_rfc3339()),
                ],
            )
            .map_err(to_storage)?;
            Ok(())
        })
    }

    fn consolidate(&self, config: &ConsolidationConfig) -> Result<ConsolidationReport> {
        self.with_conn(|conn| {
            if config.max_recent_states > 0 {
                let threshold: Option<String> = conn
                    .query_row(
                        "SELECT timestamp FROM timeline_events ORDER BY timestamp DESC LIMIT 1 OFFSET ?1",
                        params![config.max_recent_states as i64],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(to_storage)?;
                if let Some(threshold) = threshold {
                    let compacted = conn
                        .execute("DELETE FROM timeline_events WHERE timestamp < ?1", params![threshold])
                        .map_err(to_storage)?;
                    return Ok(ConsolidationReport {
                        merged_knowledge: 0,
                        promoted_rules: 0,
                        compacted_events: compacted,
                    });
                }
            }
            Ok(ConsolidationReport::default())
        })
    }

    fn backup(&self, path: &Path) -> Result<()> {
        self.with_conn(|conn| {
            conn.backup(DatabaseName::Main, path, None)
                .map_err(to_storage)?;
            Ok(())
        })
    }
}

fn register_sqlite_vec() {
    REGISTER_VEC.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite3_vec_init as *const (),
        )));
    });
}

fn to_storage(error: impl ToString) -> KernelError {
    KernelError::Storage(error.to_string())
}

fn count_table(conn: &Connection, table: &str) -> Result<usize> {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get::<_, i64>(0)
    })
    .map(|value| value as usize)
    .map_err(to_storage)
}

fn parse_datetime(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn embedding_json(values: &[f32]) -> String {
    let output = values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("[{output}]")
}

fn sanitize_fts_query(query: &str) -> Option<String> {
    let tokens = query
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

fn row_to_experience(row: &rusqlite::Row<'_>) -> rusqlite::Result<Experience> {
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

fn row_to_knowledge(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeNode> {
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

fn row_to_timeline_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimelineEvent> {
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

enum Embedder {
    Fast(Box<TextEmbedding>),
    Fallback,
}

impl Embedder {
    fn new() -> Self {
        let mut options = InitOptions::default();
        options.model_name = EmbeddingModel::BGESmallENV15;
        options.show_download_progress = false;
        match TextEmbedding::try_new(options) {
            Ok(model) => Self::Fast(Box::new(model)),
            Err(_) => Self::Fallback,
        }
    }

    fn embed(&self, input: &str) -> Vec<f32> {
        match self {
            Self::Fast(model) => model
                .embed(vec![input], None)
                .ok()
                .and_then(|vectors| vectors.into_iter().next())
                .unwrap_or_else(|| hashed_embedding(input)),
            Self::Fallback => hashed_embedding(input),
        }
    }
}

fn hashed_embedding(input: &str) -> Vec<f32> {
    let digest = blake3::hash(input.as_bytes());
    let bytes = digest.as_bytes();
    (0..384)
        .map(|index| {
            let byte = bytes[index % bytes.len()];
            (byte as f32 / 255.0) * 2.0 - 1.0
        })
        .collect()
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestFile {
    agent_id: String,
    domain: String,
    status: String,
    description: String,
    created_at: String,
}

pub fn write_manifest(path: PathBuf, manifest: &AgentManifest) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| KernelError::Configuration("manifest path missing parent".to_string()))?;
    std::fs::create_dir_all(parent)?;
    let file = ManifestFile {
        agent_id: manifest.agent_id.0.clone(),
        domain: manifest.domain.clone(),
        status: format!("{:?}", manifest.status),
        description: manifest.description.clone(),
        created_at: manifest.created_at.to_rfc3339(),
    };
    std::fs::write(path, toml::to_string_pretty(&file).map_err(to_storage)?)
        .map_err(|error| KernelError::Storage(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn migrations_create_schema() {
        let memory = SqliteMemory::open_in_memory().unwrap();
        let events = memory.recent_states(10).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn timeline_events_persist() {
        let memory = SqliteMemory::open_in_memory().unwrap();
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({"test": true}),
        };
        memory.append_timeline_event(&event).unwrap();
        let events = memory.recent_states(10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event_type,
            TimelineEventType::TaskReceived
        ));
    }

    #[test]
    fn knowledge_and_experience_can_be_recalled() {
        let memory = SqliteMemory::open_in_memory().unwrap();
        let knowledge = KnowledgeNode {
            id: None,
            category: "lesson".to_string(),
            content: "Use verification after file writes.".to_string(),
            confidence: 0.9,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: json!({}),
        };
        memory.store_knowledge(&knowledge).unwrap();
        let experience = Experience {
            id: None,
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            intent_id: IntentId::new(),
            action_summary: "write file".to_string(),
            outcome: "success".to_string(),
            utility: 0.8,
            created_at: Utc::now(),
            metadata: json!({}),
        };
        memory.record_experience(&experience).unwrap();
        assert!(
            !memory
                .recall_knowledge("verification", 5)
                .unwrap()
                .is_empty()
        );
        assert!(!memory.recall_experiences("write", 5).unwrap().is_empty());
    }

    #[test]
    fn utility_update_changes_ranking() {
        let memory = SqliteMemory::open_in_memory().unwrap();
        let first = Experience {
            id: Some(ExperienceId::new()),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            intent_id: IntentId::new(),
            action_summary: "run task".to_string(),
            outcome: "old".to_string(),
            utility: 0.1,
            created_at: Utc::now(),
            metadata: json!({}),
        };
        let second = Experience {
            id: Some(ExperienceId::new()),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            intent_id: IntentId::new(),
            action_summary: "run task".to_string(),
            outcome: "new".to_string(),
            utility: 0.2,
            created_at: Utc::now(),
            metadata: json!({}),
        };
        let first_id = memory.record_experience(&first).unwrap();
        memory.record_experience(&second).unwrap();
        memory.update_utility(first_id, 0.9).unwrap();
        let recalled = memory.recall_experiences("run", 5).unwrap();
        assert_eq!(recalled[0].outcome, "old");
    }

    #[test]
    fn backup_succeeds() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("agent.db");
        let backup = dir.path().join("backup.db");
        let memory = SqliteMemory::open(&db).unwrap();
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({}),
        };
        memory.append_timeline_event(&event).unwrap();
        memory.backup(&backup).unwrap();
        assert!(backup.exists());
    }
}
