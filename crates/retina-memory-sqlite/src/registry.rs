use chrono::Utc;
use retina_types::*;

pub fn agent_card_from_manifest(manifest: &AgentManifest) -> AgentCard {
    AgentCard {
        agent_id: manifest.agent_id.clone(),
        domain: manifest.domain.clone(),
        description: manifest.description.clone(),
        capabilities: manifest.capabilities.clone(),
        status: manifest.status.clone(),
        lifecycle_phase: manifest.lifecycle.phase.clone(),
        last_active_at: manifest.lifecycle.last_active_at,
    }
}

pub fn registry_snapshot(manifests: Vec<AgentManifest>) -> AgentRegistrySnapshot {
    let mut active_agents = Vec::new();
    let mut archived_agents = Vec::new();

    for manifest in manifests {
        let card = agent_card_from_manifest(&manifest);
        if matches!(card.status, AgentStatus::Archived) {
            archived_agents.push(card);
        } else {
            active_agents.push(card);
        }
    }

    AgentRegistrySnapshot {
        updated_at: Utc::now(),
        active_agents,
        archived_agents,
    }
}
