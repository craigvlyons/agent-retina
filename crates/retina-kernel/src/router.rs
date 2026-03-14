use retina_types::{
    AgentCard, AgentRegistrySnapshot, RoutingAssessment, RoutingCandidate, RoutingDecision, Task,
};

const ROUTE_MATCH_THRESHOLD: f64 = 0.75;
const SPAWN_MATCH_THRESHOLD: f64 = 0.55;

#[derive(Clone, Debug)]
pub struct Router {
    network_enabled: bool,
    registry: AgentRegistrySnapshot,
}

impl Router {
    pub fn v1(registry: AgentRegistrySnapshot) -> Self {
        Self {
            network_enabled: false,
            registry,
        }
    }

    pub fn route_task(&self, task: &Task) -> RoutingAssessment {
        let recommended = self.recommended_decision(task);
        let effective_decision = if self.network_enabled {
            recommended.clone()
        } else {
            RoutingDecision::HandleDirectly
        };
        let rationale = if self.network_enabled {
            build_rationale(&recommended)
        } else {
            format!(
                "network routing disabled in v1; {}",
                build_rationale(&recommended)
            )
        };

        RoutingAssessment {
            effective_decision,
            recommended_decision: recommended,
            candidates: self.candidates(task),
            rationale,
            network_enabled: self.network_enabled,
        }
    }

    fn recommended_decision(&self, task: &Task) -> RoutingDecision {
        let active_candidates = self
            .registry
            .active_agents
            .iter()
            .filter(|card| !is_root_orchestrator(card))
            .map(|card| (card, capability_match(task, card)))
            .filter(|(_, score)| *score >= ROUTE_MATCH_THRESHOLD)
            .max_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some((card, _)) = active_candidates {
            return RoutingDecision::RouteToExisting(card.agent_id.clone());
        }

        let archived_candidates = self
            .registry
            .archived_agents
            .iter()
            .filter(|card| !is_root_orchestrator(card))
            .map(|card| (card, capability_match(task, card)))
            .filter(|(_, score)| *score >= ROUTE_MATCH_THRESHOLD)
            .max_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some((card, _)) = archived_candidates {
            return RoutingDecision::Reactivate(card.agent_id.clone());
        }

        if likely_specialist_task(task) {
            return RoutingDecision::SpawnSpecialist {
                domain: infer_domain(task),
                capability: capability_summary(task),
            };
        }

        RoutingDecision::HandleDirectly
    }

    fn candidates(&self, task: &Task) -> Vec<RoutingCandidate> {
        let mut candidates = self
            .registry
            .active_agents
            .iter()
            .chain(self.registry.archived_agents.iter())
            .filter(|card| !is_root_orchestrator(card))
            .map(|card| RoutingCandidate {
                agent_id: card.agent_id.clone(),
                domain: card.domain.clone(),
                status: card.status.clone(),
                capability_match: capability_match(task, card),
                reason: candidate_reason(task, card),
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .capability_match
                .partial_cmp(&left.capability_match)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates
    }
}

fn build_rationale(decision: &RoutingDecision) -> String {
    match decision {
        RoutingDecision::HandleDirectly => {
            "task looks like direct worker work; keep it on the current agent".to_string()
        }
        RoutingDecision::RouteToExisting(agent_id) => {
            format!(
                "an active specialist already matches this task: {}",
                agent_id.0
            )
        }
        RoutingDecision::Reactivate(agent_id) => {
            format!(
                "an archived specialist matches this task and could be reactivated: {}",
                agent_id.0
            )
        }
        RoutingDecision::SpawnSpecialist { domain, capability } => format!(
            "task looks domain-specific and recurring enough to justify a `{domain}` specialist ({capability})"
        ),
    }
}

fn capability_match(task: &Task, card: &AgentCard) -> f64 {
    let task_lower = task.description.to_lowercase();
    let haystack = format!(
        "{} {} {}",
        card.domain.to_lowercase(),
        card.description.to_lowercase(),
        card.capabilities.join(" ").to_lowercase()
    );
    let tokens = tokenize(&task.description);
    if tokens.is_empty() {
        return 0.0;
    }

    let matched = tokens
        .iter()
        .filter(|token| haystack.contains(token.as_str()))
        .count() as f64;
    let token_score = matched / tokens.len() as f64;
    let domain_bonus = if task_lower.contains(card.domain.as_str()) {
        0.35
    } else {
        0.0
    };
    let capability_bonus = card
        .capabilities
        .iter()
        .filter(|capability| task_lower.contains(capability.to_lowercase().as_str()))
        .count() as f64
        * 0.15;

    (token_score + domain_bonus + capability_bonus).min(1.0)
}

fn tokenize(input: &str) -> Vec<String> {
    input
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| part.len() > 2)
        .map(ToString::to_string)
        .collect()
}

fn candidate_reason(_task: &Task, card: &AgentCard) -> String {
    format!(
        "matched task terms against domain `{}` and capabilities `{}`",
        card.domain,
        card.capabilities.join(", ")
    )
}

fn is_root_orchestrator(card: &AgentCard) -> bool {
    card.agent_id.0 == "root" || card.domain == "orchestrator"
}

fn likely_specialist_task(task: &Task) -> bool {
    let lower = task.description.to_lowercase();
    let domain_specific = [
        "email", "browser", "research", "hardware", "device", "deploy", "ops", "ci", "form",
        "invoice", "inbox", "monitor",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword));
    let recurring = [
        "watch",
        "monitor",
        "every",
        "whenever",
        "check my",
        "keep track",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword));
    domain_specific || (recurring && tokenize(&lower).len() >= 4)
}

fn infer_domain(task: &Task) -> String {
    let lower = task.description.to_lowercase();
    for (keyword, domain) in [
        ("email", "email"),
        ("browser", "browser"),
        ("web", "research"),
        ("research", "research"),
        ("hardware", "hardware"),
        ("device", "hardware"),
        ("deploy", "ops"),
        ("ci", "ops"),
        ("invoice", "email"),
        ("form", "browser"),
    ] {
        if lower.contains(keyword) {
            return domain.to_string();
        }
    }
    "generalist".to_string()
}

fn capability_summary(task: &Task) -> String {
    let lower = task.description.to_lowercase();
    let tokens = tokenize(&lower);
    let summary = tokens.into_iter().take(8).collect::<Vec<_>>().join(" ");
    if summary.is_empty() {
        "domain-specific recurring work".to_string()
    } else if capability_strength(&lower) >= SPAWN_MATCH_THRESHOLD {
        summary
    } else {
        format!("specialized support for {}", summary)
    }
}

fn capability_strength(task: &str) -> f64 {
    let total = ["email", "browser", "research", "hardware", "ops", "monitor"]
        .iter()
        .filter(|keyword| task.contains(**keyword))
        .count();
    total as f64 / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use retina_types::{AgentLifecyclePhase, AgentStatus};

    fn specialist_card(
        agent_id: &str,
        domain: &str,
        capabilities: &[&str],
        status: AgentStatus,
    ) -> AgentCard {
        AgentCard {
            agent_id: retina_types::AgentId(agent_id.to_string()),
            domain: domain.to_string(),
            description: format!("{domain} specialist"),
            capabilities: capabilities.iter().map(|value| value.to_string()).collect(),
            status: status.clone(),
            lifecycle_phase: if matches!(status, AgentStatus::Archived) {
                AgentLifecyclePhase::Archived
            } else {
                AgentLifecyclePhase::Ready
            },
            last_active_at: Some(Utc::now()),
        }
    }

    #[test]
    fn v1_router_keeps_effective_decision_local_but_records_recommendation() {
        let router = Router::v1(AgentRegistrySnapshot {
            updated_at: Utc::now(),
            active_agents: vec![specialist_card(
                "email-a1",
                "email",
                &["imap", "inbox", "invoice"],
                AgentStatus::Idle,
            )],
            archived_agents: vec![],
        });
        let assessment = router.route_task(&Task::new(
            retina_types::AgentId("root".to_string()),
            "check my email for invoices",
        ));
        assert!(matches!(
            assessment.effective_decision,
            RoutingDecision::HandleDirectly
        ));
        assert!(matches!(
            assessment.recommended_decision,
            RoutingDecision::RouteToExisting(_)
        ));
        assert!(!assessment.candidates.is_empty());
    }
}
