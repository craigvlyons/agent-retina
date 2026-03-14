use blake3::hash;
use chrono::{DateTime, Utc};
use retina_types::{Action, ConsolidationConfig, Experience, ReflexiveRule, RuleCondition, RuleId};
use serde_json::Value;
use std::collections::HashMap;

use crate::retrieval::{experience_task_text, normalize_text, task_signature};

#[derive(Clone, Debug)]
pub(crate) struct ExperiencePattern {
    pub key: String,
    pub task_text: String,
    pub task_signature: String,
    pub action: Action,
    pub action_summary: String,
    pub observation_count: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub average_utility: f64,
    pub confidence: f64,
    pub last_seen: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
struct PatternAccumulator {
    task_text: String,
    task_signature: String,
    action: Option<Action>,
    action_summary: String,
    observation_count: usize,
    success_count: usize,
    failure_count: usize,
    utility_total: f64,
    last_seen: Option<DateTime<Utc>>,
}

pub(crate) fn build_experience_patterns(
    experiences: &[Experience],
    config: &ConsolidationConfig,
) -> Vec<ExperiencePattern> {
    let mut grouped: HashMap<String, PatternAccumulator> = HashMap::new();

    for experience in experiences {
        let Some(task_text) = experience_task_text(experience) else {
            continue;
        };
        let Some(action) = experience_action(experience) else {
            continue;
        };
        if !is_promotable_action(&action) {
            continue;
        }

        let normalized_task = normalize_text(&task_text);
        if normalized_task.is_empty() {
            continue;
        }
        let task_signature = task_signature(&normalized_task);

        let key = pattern_key(&task_signature, &experience.action_summary);
        let entry = grouped.entry(key).or_default();
        entry.task_text = task_text;
        entry.task_signature = task_signature;
        entry.action = Some(action);
        entry.action_summary = experience.action_summary.clone();
        entry.observation_count += 1;
        if experience.utility >= config.min_success_utility {
            entry.success_count += 1;
        } else if experience.utility < 0.0 {
            entry.failure_count += 1;
        }
        entry.utility_total += experience.utility;
        entry.last_seen = Some(
            entry
                .last_seen
                .map(|value| value.max(experience.created_at))
                .unwrap_or(experience.created_at),
        );
    }

    grouped
        .into_iter()
        .filter_map(|(key, item)| {
            let action = item.action?;
            if item.observation_count < config.min_successful_repeats || item.success_count == 0 {
                return None;
            }

            let average_utility = item.utility_total / item.observation_count as f64;
            let repeat_score = (item.observation_count as f64
                / (config.min_successful_repeats as f64 + 2.0))
                .min(1.0);
            let success_ratio = item.success_count as f64 / item.observation_count as f64;
            let confidence = ((average_utility.clamp(0.0, 1.0) * 0.45)
                + (repeat_score * 0.20)
                + (success_ratio * 0.35))
                .clamp(0.0, 0.98);

            Some(ExperiencePattern {
                key,
                task_text: item.task_text,
                task_signature: item.task_signature,
                action,
                action_summary: item.action_summary,
                observation_count: item.observation_count,
                success_count: item.success_count,
                failure_count: item.failure_count,
                average_utility,
                confidence,
                last_seen: item.last_seen.unwrap_or_else(Utc::now),
            })
        })
        .collect()
}

pub(crate) fn knowledge_content(pattern: &ExperiencePattern) -> String {
    format!(
        "For tasks like \"{}\", prefer {} because it succeeded {} out of {} observed times.",
        pattern.task_text, pattern.action_summary, pattern.success_count, pattern.observation_count
    )
}

pub(crate) fn rule_name(pattern: &ExperiencePattern) -> String {
    format!("consolidated:{}", pattern.key)
}

pub(crate) fn should_promote_rule(
    pattern: &ExperiencePattern,
    config: &ConsolidationConfig,
) -> bool {
    pattern.confidence >= config.min_rule_confidence
        && pattern.success_count > pattern.failure_count
}

pub(crate) fn rule_matches_pattern(rule: &ReflexiveRule, pattern: &ExperiencePattern) -> bool {
    rule.id.is_some()
        && rule.name == rule_name(pattern)
        && matches!(
            &rule.condition,
            RuleCondition::TaskContains(value) if value == &pattern.task_text
        )
}

pub(crate) fn metadata_key(value: &Value) -> Option<&str> {
    value
        .get("consolidation")
        .and_then(|item| item.get("key"))
        .and_then(Value::as_str)
}

pub(crate) fn metadata_success_count(value: &Value) -> usize {
    value
        .get("consolidation")
        .and_then(|item| item.get("success_count"))
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or_default()
}

pub(crate) fn pattern_metadata(pattern: &ExperiencePattern) -> Value {
    serde_json::json!({
        "consolidation": {
            "key": pattern.key,
            "task": pattern.task_text,
            "task_signature": pattern.task_signature,
            "action_summary": pattern.action_summary,
            "observation_count": pattern.observation_count,
            "success_count": pattern.success_count,
            "failure_count": pattern.failure_count,
            "average_utility": pattern.average_utility,
            "confidence": pattern.confidence,
            "last_seen": pattern.last_seen.to_rfc3339(),
            "source": "experience_pattern"
        }
    })
}

fn pattern_key(task_signature: &str, action_summary: &str) -> String {
    hash(format!("{task_signature}::{action_summary}").as_bytes())
        .to_hex()
        .to_string()
}

fn experience_action(experience: &Experience) -> Option<Action> {
    experience
        .metadata
        .get("action")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn is_promotable_action(action: &Action) -> bool {
    !matches!(
        action,
        Action::Respond { .. } | Action::RecordNote { .. } | Action::InspectWorkingDirectory { .. }
    )
}

#[allow(dead_code)]
pub(crate) fn existing_rule_id(rule: &ReflexiveRule) -> Option<RuleId> {
    rule.id.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use retina_types::{ActionId, Experience, ExperienceId, IntentId, SessionId, TaskId};

    #[test]
    fn builds_patterns_from_repeated_successful_experiences() {
        let action = Action::ReadFile {
            id: ActionId::new(),
            path: "startup.md".into(),
            max_bytes: None,
        };
        let experiences = (0..3)
            .map(|_| Experience {
                id: Some(ExperienceId::new()),
                session_id: SessionId::new(),
                task_id: TaskId::new(),
                intent_id: IntentId::new(),
                action_summary: "read_file:startup.md".to_string(),
                outcome: "ChangedAsExpected".to_string(),
                utility: 1.0,
                created_at: Utc::now(),
                metadata: serde_json::json!({
                    "task": "read startup.md",
                    "action": action,
                }),
            })
            .collect::<Vec<_>>();

        let patterns = build_experience_patterns(&experiences, &ConsolidationConfig::default());
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].success_count, 3);
        assert!(patterns[0].confidence >= 0.8);
    }
}
