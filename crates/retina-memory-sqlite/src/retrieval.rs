use chrono::{DateTime, Utc};
use retina_types::{Experience, KnowledgeNode};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashSet;

pub(crate) fn rank_experiences(
    query: &str,
    experiences: Vec<Experience>,
    limit: usize,
) -> Vec<Experience> {
    let normalized_query = normalize_text(query);
    let query_tokens = tokens(&normalized_query);
    let mut scored = experiences
        .into_iter()
        .map(|experience| {
            let score = score_experience(&normalized_query, &query_tokens, &experience);
            (score, experience)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.1.created_at.cmp(&left.1.created_at))
    });

    scored
        .into_iter()
        .take(limit)
        .map(|(_, experience)| experience)
        .collect()
}

pub(crate) fn rerank_knowledge(
    query: &str,
    knowledge: Vec<KnowledgeNode>,
    limit: usize,
) -> Vec<KnowledgeNode> {
    let normalized_query = normalize_text(query);
    let query_tokens = tokens(&normalized_query);
    let mut scored = knowledge
        .into_iter()
        .map(|node| {
            let score = score_knowledge(&normalized_query, &query_tokens, &node);
            (score, node)
        })
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                right
                    .1
                    .updated_at
                    .cmp(&left.1.updated_at)
                    .then_with(|| right.1.created_at.cmp(&left.1.created_at))
            })
    });

    scored
        .into_iter()
        .take(limit)
        .map(|(_, node)| node)
        .collect()
}

pub(crate) fn experience_task_text(experience: &Experience) -> Option<String> {
    experience
        .metadata
        .get("task")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn normalize_text(input: &str) -> String {
    input
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn task_signature(input: &str) -> String {
    let mut values = tokens(input)
        .into_iter()
        .filter(|token| !is_noise_token(token))
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    if values.is_empty() {
        normalize_text(input)
    } else {
        values.join(" ")
    }
}

fn score_experience(
    normalized_query: &str,
    query_tokens: &[String],
    experience: &Experience,
) -> f64 {
    let task_text = experience_task_text(experience).unwrap_or_default();
    let normalized_task = normalize_text(&task_text);
    let action_summary = normalize_text(&experience.action_summary);
    let outcome = normalize_text(&experience.outcome);

    let task_overlap = overlap_score(query_tokens, &tokens(&normalized_task));
    let action_overlap = overlap_score(query_tokens, &tokens(&action_summary));
    let query_signature = task_signature(normalized_query);
    let task_signature = task_signature(&normalized_task);
    let utility_score = ((experience.utility + 1.0) / 2.0).clamp(0.0, 1.0);
    let recency_score = recency_score(experience.created_at);

    let mut score = 0.0;
    if !normalized_task.is_empty() {
        score += task_overlap * 4.0;
        if normalized_task.contains(normalized_query) || normalized_query.contains(&normalized_task) {
            score += 1.5;
        }
        if !query_signature.is_empty() && query_signature == task_signature {
            score += 2.0;
        }
    }
    if !action_summary.is_empty() {
        score += action_overlap * 2.0;
        if action_summary.contains(normalized_query) {
            score += 0.75;
        }
    }
    if outcome.contains("changedasexpected") || outcome.contains("success") {
        score += 0.25;
    }

    score + (utility_score * 1.5) + recency_score
}

fn score_knowledge(normalized_query: &str, query_tokens: &[String], node: &KnowledgeNode) -> f64 {
    let normalized_content = normalize_text(&node.content);
    let metadata_task = node
        .metadata
        .get("consolidation")
        .and_then(|value| value.get("task"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let normalized_task = normalize_text(metadata_task);
    let content_overlap = overlap_score(query_tokens, &tokens(&normalized_content));
    let task_overlap = overlap_score(query_tokens, &tokens(&normalized_task));

    let mut score = (node.confidence.clamp(0.0, 1.0) * 2.0) + content_overlap * 2.5 + task_overlap * 3.0;
    if normalized_content.contains(normalized_query) || normalized_task.contains(normalized_query) {
        score += 1.0;
    }
    score
}

fn overlap_score(left: &[String], right: &[String]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left = left.iter().cloned().collect::<HashSet<_>>();
    let right = right.iter().cloned().collect::<HashSet<_>>();
    let intersection = left.intersection(&right).count() as f64;
    let union = left.union(&right).count() as f64;
    if union == 0.0 { 0.0 } else { intersection / union }
}

fn tokens(input: &str) -> Vec<String> {
    input
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn is_noise_token(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "a"
            | "an"
            | "and"
            | "then"
            | "for"
            | "with"
            | "into"
            | "from"
            | "that"
            | "this"
            | "what"
            | "tell"
            | "show"
            | "read"
            | "open"
            | "find"
            | "list"
            | "search"
            | "look"
            | "use"
            | "please"
            | "me"
            | "my"
            | "is"
            | "are"
            | "to"
            | "of"
            | "in"
            | "on"
    ) || token.len() <= 2
    }

fn recency_score(created_at: DateTime<Utc>) -> f64 {
    let age_seconds = (Utc::now() - created_at).num_seconds().max(0) as f64;
    1.0 / (1.0 + (age_seconds / 3600.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use retina_types::{ExperienceId, IntentId, SessionId, TaskId};
    use serde_json::json;

    #[test]
    fn similar_task_phrasing_can_recall_experience() {
        let experience = Experience {
            id: Some(ExperienceId::new()),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            intent_id: IntentId::new(),
            action_summary: "read_file:/tmp/resume.md".to_string(),
            outcome: "success".to_string(),
            utility: 0.8,
            created_at: Utc::now(),
            metadata: json!({
                "task": "read the craig lyons resume markdown file"
            }),
        };

        let ranked = rank_experiences("open craig lyons resume.md and summarize it", vec![experience], 3);
        assert_eq!(ranked.len(), 1);
    }
}
