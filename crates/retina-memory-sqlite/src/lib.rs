// File boundary: keep lib.rs focused on top-level storage wiring and trait
// implementation entry points. Move new helpers and feature logic into modules.
mod consolidation;
mod embedder;
mod manifest;
mod registry;
mod retrieval;
mod storage;

use chrono::Utc;
use consolidation::{
    build_experience_patterns, knowledge_content, metadata_key, metadata_success_count,
    pattern_metadata, rule_matches_pattern, rule_name, should_promote_rule,
};
use embedder::Embedder;
pub use manifest::write_manifest;
use refinery::embed_migrations;
use retina_traits::Memory;
use retina_types::*;
use retrieval::{rank_experiences, rerank_knowledge};
use rusqlite::{Connection, DatabaseName, OptionalExtension, params};
use serde_json::json;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use storage::{
    embedding_json, load_knowledge_nodes, load_recent_experiences, load_rules, parse_datetime,
    persist_knowledge, persist_rule, register_sqlite_vec, row_to_experience, row_to_knowledge,
    row_to_timeline_event, sanitize_fts_query, to_storage,
};

embed_migrations!("migrations");

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| KernelError::Storage("sqlite connection mutex poisoned".to_string()))?;
        func(&conn)
    }

    fn embed_text(&self, input: &str) -> Vec<f32> {
        self.embedder.embed(input)
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
        self.with_conn(|conn| persist_knowledge(conn, &self.embedder, node))
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
        self.with_conn(|conn| persist_rule(conn, rule))
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
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, task_id, intent_id, action_summary, outcome, utility, created_at, metadata
                     FROM experiences
                     ORDER BY created_at DESC
                     LIMIT ?1",
                )
                .map_err(to_storage)?;
            let rows = stmt
                .query_map(params![(limit.max(8) * 8) as i64], row_to_experience)
                .map_err(to_storage)?;
            let experiences = rows
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(to_storage)?;
            Ok(rank_experiences(query, experiences, limit))
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
            Ok(rerank_knowledge(query, output, limit))
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
            let experiences = load_recent_experiences(conn, 256)?;
            let patterns = build_experience_patterns(&experiences, config);
            let mut knowledge = load_knowledge_nodes(conn)?;
            let mut rules = load_rules(conn, false)?;
            let mut report = ConsolidationReport::default();

            for pattern in patterns {
                let metadata = pattern_metadata(&pattern);
                let confidence = pattern.confidence;
                let existing_knowledge = knowledge
                    .iter()
                    .find(|node| metadata_key(&node.metadata) == Some(pattern.key.as_str()))
                    .cloned();
                let should_update_knowledge = existing_knowledge
                    .as_ref()
                    .map(|node| {
                        metadata_success_count(&node.metadata) < pattern.success_count
                            || node.confidence + 0.05 < confidence
                    })
                    .unwrap_or(true);

                if should_update_knowledge {
                    if let Some(existing) = existing_knowledge {
                        if let Some(id) = existing.id.clone() {
                            conn.execute(
                                "UPDATE knowledge
                                 SET content = ?2, confidence = ?3, metadata = ?4, updated_at = ?5
                                 WHERE id = ?1",
                                params![
                                    id.0,
                                    knowledge_content(&pattern),
                                    confidence.max(existing.confidence),
                                    serde_json::to_string(&metadata).map_err(to_storage)?,
                                    Utc::now().to_rfc3339(),
                                ],
                            )
                            .map_err(to_storage)?;
                        }
                    } else {
                        let node = KnowledgeNode {
                            id: None,
                            category: "pattern".to_string(),
                            content: knowledge_content(&pattern),
                            confidence,
                            created_at: Utc::now(),
                            updated_at: Utc::now(),
                            metadata: metadata.clone(),
                        };
                        let id = persist_knowledge(conn, &self.embedder, &node)?;
                        let mut stored = node;
                        stored.id = Some(id);
                        knowledge.push(stored);
                    }
                    report.merged_knowledge += 1;
                }

                let existing_rule = rules
                    .iter()
                    .find(|rule| rule_matches_pattern(rule, &pattern))
                    .cloned();
                let desired_active = should_promote_rule(&pattern, config);
                let should_update_rule = existing_rule
                    .as_ref()
                    .map(|rule| {
                        rule.active != desired_active
                            || (desired_active && rule.confidence + 0.05 < confidence)
                            || (!desired_active && rule.confidence > confidence + 0.05)
                    })
                    .unwrap_or(desired_active);
                if should_update_rule {
                    let rule = ReflexiveRule {
                        id: existing_rule.as_ref().and_then(|value| value.id.clone()),
                        name: rule_name(&pattern),
                        condition: RuleCondition::TaskContains(pattern.task_text.clone()),
                        action: RuleAction::UseAction(pattern.action.clone()),
                        confidence,
                        active: desired_active,
                        last_fired: None,
                    };
                    let id = persist_rule(conn, &rule)?;
                    if let Some(existing_rule) = existing_rule {
                        if let Some(position) =
                            rules.iter().position(|item| item.name == existing_rule.name)
                        {
                            rules[position] = ReflexiveRule {
                                id: Some(id),
                                ..rule
                            };
                        }
                    } else {
                        rules.push(ReflexiveRule {
                            id: Some(id),
                            ..rule
                        });
                    }
                    if desired_active {
                        report.promoted_rules += 1;
                    }
                }
            }

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
                        .execute(
                            "DELETE FROM timeline_events WHERE timestamp <= ?1",
                            params![threshold],
                        )
                        .map_err(to_storage)?;
                    conn.execute(
                        "DELETE FROM state_log WHERE timestamp <= ?1",
                        params![threshold],
                    )
                        .map_err(to_storage)?;
                    report.compacted_events = compacted;
                }
            }

            if let Some(days) = config.stale_knowledge_days {
                let threshold = Utc::now() - chrono::Duration::days(days as i64);
                let decayed = conn
                    .execute(
                        "UPDATE knowledge
                         SET confidence = CASE
                                WHEN confidence > 0.10 THEN MAX(confidence * 0.9, 0.10)
                                ELSE confidence
                             END,
                             updated_at = ?2
                         WHERE updated_at < ?1",
                        params![threshold.to_rfc3339(), Utc::now().to_rfc3339()],
                    )
                    .map_err(to_storage)?;
                report.decayed_knowledge = decayed;
            }

            if config.optimize_after_cleanup {
                conn.execute("INSERT INTO knowledge_fts(knowledge_fts) VALUES('optimize')", [])
                    .map_err(to_storage)?;
                conn.execute_batch("PRAGMA optimize;").map_err(to_storage)?;
                report.optimized = true;
            }
            Ok(report)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
        result.unwrap_or_else(|error| panic!("test operation failed: {error}"))
    }

    fn must_tempdir() -> tempfile::TempDir {
        tempdir().unwrap_or_else(|error| panic!("failed to create tempdir: {error}"))
    }

    #[test]
    fn migrations_create_schema() {
        let memory = must(SqliteMemory::open_in_memory());
        let events = must(memory.recent_states(10));
        assert!(events.is_empty());
    }

    #[test]
    fn timeline_events_persist() {
        let memory = must(SqliteMemory::open_in_memory());
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
        must(memory.append_timeline_event(&event));
        let events = must(memory.recent_states(10));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event_type,
            TimelineEventType::TaskReceived
        ));
    }

    #[test]
    fn knowledge_and_experience_can_be_recalled() {
        let memory = must(SqliteMemory::open_in_memory());
        let knowledge = KnowledgeNode {
            id: None,
            category: "lesson".to_string(),
            content: "Use verification after file writes.".to_string(),
            confidence: 0.9,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: json!({}),
        };
        must(memory.store_knowledge(&knowledge));
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
        must(memory.record_experience(&experience));
        assert!(!must(memory.recall_knowledge("verification", 5)).is_empty());
        assert!(!must(memory.recall_experiences("write", 5)).is_empty());
    }

    #[test]
    fn hyphenated_recall_queries_do_not_break_fts() {
        let memory = must(SqliteMemory::open_in_memory());
        let knowledge = KnowledgeNode {
            id: None,
            category: "lesson".to_string(),
            content: "Use retina cleanup check filenames safely.".to_string(),
            confidence: 0.9,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: json!({}),
        };
        must(memory.store_knowledge(&knowledge));
        assert!(must(memory.recall_knowledge("retina-cleanup-check.txt", 5)).len() <= 5);
    }

    #[test]
    fn utility_update_changes_ranking() {
        let memory = must(SqliteMemory::open_in_memory());
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
        let first_id = must(memory.record_experience(&first));
        must(memory.record_experience(&second));
        must(memory.update_utility(first_id, 0.9));
        let recalled = must(memory.recall_experiences("run", 5));
        assert_eq!(recalled[0].outcome, "old");
    }

    #[test]
    fn similar_task_phrasing_reuses_prior_experience() {
        let memory = must(SqliteMemory::open_in_memory());
        must(memory.record_experience(&Experience {
            id: None,
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            intent_id: IntentId::new(),
            action_summary:
                "read_file:/Users/macc/Desktop/resume/Craig Lyons resume.md".to_string(),
            outcome: "success".to_string(),
            utility: 0.8,
            created_at: Utc::now(),
            metadata: json!({
                "task": "read the craig lyons resume markdown file and summarize it",
                "action": Action::ReadFile {
                    id: ActionId::new(),
                    path: "/Users/macc/Desktop/resume/Craig Lyons resume.md".into(),
                    max_bytes: None,
                }
            }),
        }));

        let recalled =
            must(memory.recall_experiences("find craig lyons resume.md and tell me about it", 5));
        assert_eq!(recalled.len(), 1);
    }

    #[test]
    fn backup_succeeds() {
        let dir = must_tempdir();
        let db = dir.path().join("agent.db");
        let backup = dir.path().join("backup.db");
        let memory = must(SqliteMemory::open(&db));
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
        must(memory.append_timeline_event(&event));
        must(memory.backup(&backup));
        assert!(backup.exists());
    }

    #[test]
    fn manifests_persist_lifecycle_and_registry_snapshot() {
        let memory = must(SqliteMemory::open_in_memory());
        let now = Utc::now();
        must(memory.save_manifest(&AgentManifest {
            agent_id: AgentId("root".to_string()),
            domain: "orchestrator".to_string(),
            status: AgentStatus::Idle,
            description: "root".to_string(),
            created_at: now,
            updated_at: now,
            parent_agent_id: None,
            capabilities: vec!["cli".to_string()],
            authority: AgentAuthority::default(),
            lifecycle: AgentLifecycle::ready(),
            budget: AgentBudget::default(),
        }));
        let mut archived_lifecycle = AgentLifecycle::ready();
        archived_lifecycle.transition(
            AgentLifecyclePhase::Archived,
            now,
            Some("idle timeout".to_string()),
        );
        must(memory.save_manifest(&AgentManifest {
            agent_id: AgentId("research-a1".to_string()),
            domain: "research".to_string(),
            status: AgentStatus::Archived,
            description: "research specialist".to_string(),
            created_at: now,
            updated_at: now,
            parent_agent_id: Some(AgentId("root".to_string())),
            capabilities: vec!["web".to_string(), "documents".to_string()],
            authority: AgentAuthority::default(),
            lifecycle: archived_lifecycle,
            budget: AgentBudget::default(),
        }));

        let registry = must(memory.agent_registry());
        assert_eq!(registry.active_agents.len(), 1);
        assert_eq!(registry.archived_agents.len(), 1);
        assert_eq!(registry.archived_agents[0].domain, "research");
    }

    #[test]
    fn consolidation_can_decay_stale_knowledge_and_trim_old_timeline() {
        let memory = must(SqliteMemory::open_in_memory());
        let stale_time = Utc::now() - chrono::Duration::days(45);
        let recent_time = Utc::now();

        must(memory.append_timeline_event(&TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            timestamp: stale_time,
            event_type: TimelineEventType::TaskReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({}),
        }));
        must(memory.append_timeline_event(&TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            timestamp: recent_time,
            event_type: TimelineEventType::TaskCompleted,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({}),
        }));

        let stale = KnowledgeNode {
            id: None,
            category: "lesson".to_string(),
            content: "Old lesson".to_string(),
            confidence: 0.9,
            created_at: stale_time,
            updated_at: stale_time,
            metadata: json!({}),
        };
        let knowledge_id = must(memory.store_knowledge(&stale));

        let report = must(memory.consolidate(&ConsolidationConfig {
            max_recent_states: 1,
            stale_knowledge_days: Some(30),
            optimize_after_cleanup: true,
            ..ConsolidationConfig::default()
        }));

        assert_eq!(report.compacted_events, 1);
        assert_eq!(report.decayed_knowledge, 1);
        assert!(report.optimized);
        assert_eq!(must(memory.recent_states(10)).len(), 1);

        let confidence: f64 = must(memory.with_conn(|conn| {
            conn.query_row(
                "SELECT confidence FROM knowledge WHERE id = ?1",
                params![knowledge_id.0],
                |row| row.get(0),
            )
            .map_err(to_storage)
        }));
        assert!(confidence < 0.9);
    }

    #[test]
    fn consolidation_promotes_repeated_success_into_knowledge_and_rule() {
        let memory = must(SqliteMemory::open_in_memory());
        let action = Action::ReadFile {
            id: ActionId::new(),
            path: "startup.md".into(),
            max_bytes: None,
        };

        for _ in 0..3 {
            must(memory.record_experience(&Experience {
                id: None,
                session_id: SessionId::new(),
                task_id: TaskId::new(),
                intent_id: IntentId::new(),
                action_summary: "read_file:startup.md".to_string(),
                outcome: "ChangedAsExpected".to_string(),
                utility: 1.0,
                created_at: Utc::now(),
                metadata: json!({
                    "task": "read startup.md",
                    "action": action.clone(),
                }),
            }));
        }

        let report = must(memory.consolidate(&ConsolidationConfig::default()));
        assert!(report.merged_knowledge >= 1);
        assert!(report.promoted_rules >= 1);
        assert!(!must(memory.active_rules()).is_empty());
        assert!(
            must(memory.recall_knowledge("prefer read_file", 5))
                .iter()
                .any(|node| node.category == "pattern")
        );
    }

    #[test]
    fn consolidation_can_deactivate_rule_after_later_failures() {
        let memory = must(SqliteMemory::open_in_memory());
        let action = Action::ReadFile {
            id: ActionId::new(),
            path: "startup.md".into(),
            max_bytes: None,
        };

        for _ in 0..3 {
            must(memory.record_experience(&Experience {
                id: None,
                session_id: SessionId::new(),
                task_id: TaskId::new(),
                intent_id: IntentId::new(),
                action_summary: "read_file:startup.md".to_string(),
                outcome: "success".to_string(),
                utility: 0.9,
                created_at: Utc::now(),
                metadata: json!({
                    "task": "read startup.md",
                    "action": action.clone(),
                }),
            }));
        }

        must(memory.consolidate(&ConsolidationConfig::default()));
        assert_eq!(must(memory.active_rules()).len(), 1);

        for _ in 0..4 {
            must(memory.record_experience(&Experience {
                id: None,
                session_id: SessionId::new(),
                task_id: TaskId::new(),
                intent_id: IntentId::new(),
                action_summary: "read_file:startup.md".to_string(),
                outcome: "failure".to_string(),
                utility: -0.8,
                created_at: Utc::now(),
                metadata: json!({
                    "task": "open startup.md",
                    "action": action.clone(),
                }),
            }));
        }

        must(memory.consolidate(&ConsolidationConfig::default()));
        assert!(must(memory.active_rules()).is_empty());
    }
}
