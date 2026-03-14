use chrono::Utc;
use dotenvy::dotenv;
use retina_memory_sqlite::{SqliteMemory, write_manifest};
use retina_types::*;
use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

pub fn load_environment() -> Result<()> {
    static LOADED: OnceLock<()> = OnceLock::new();
    if LOADED.get().is_none() {
        let _ = dotenv();
        let _ = LOADED.set(());
    }
    Ok(())
}

pub fn init_runtime() -> Result<()> {
    load_environment()?;
    let home = retina_home()?;
    let root = home.join("root");
    let agents = home.join("agents");
    let shared = home.join("shared").join("promoted_tools");
    std::fs::create_dir_all(root.join("tools"))?;
    std::fs::create_dir_all(agents)?;
    std::fs::create_dir_all(shared)?;

    let config_path = home.join("config.toml");
    if !config_path.exists() {
        std::fs::write(
            &config_path,
            "default_agent = \"root\"\nreasoner = \"claude\"\n",
        )?;
    }

    let db_path = root.join("agent.db");
    let memory = open_memory(&db_path)?;
    let manifest = normalize_root_manifest(
        memory
            .load_manifest(&root_agent_id())?
            .unwrap_or(root_manifest()?),
    );
    memory.save_manifest(&manifest)?;
    write_manifest(root.join("manifest.toml"), &manifest)?;
    println!("Initialized Retina runtime at {}", home.display());
    Ok(())
}

pub fn open_memory(path: impl AsRef<std::path::Path>) -> Result<SqliteMemory> {
    SqliteMemory::open(path)
}

pub fn retina_home() -> Result<PathBuf> {
    load_environment()?;
    if let Ok(path) = env::var("RETINA_HOME") {
        return Ok(PathBuf::from(path));
    }
    let home = dirs::home_dir().ok_or_else(|| {
        KernelError::Configuration("could not determine home directory".to_string())
    })?;
    Ok(home.join(".retina"))
}

pub fn root_db_path() -> Result<PathBuf> {
    Ok(retina_home()?.join("root").join("agent.db"))
}

pub fn root_agent_id() -> AgentId {
    AgentId("root".to_string())
}

pub fn root_manifest() -> Result<AgentManifest> {
    let now = Utc::now();
    Ok(AgentManifest {
        agent_id: root_agent_id(),
        domain: "orchestrator".to_string(),
        status: AgentStatus::Idle,
        description: "Retina root agent running in independent v1 mode.".to_string(),
        created_at: now,
        updated_at: now,
        parent_agent_id: None,
        capabilities: vec![
            "cli".to_string(),
            "filesystem".to_string(),
            "search".to_string(),
            "command".to_string(),
            "memory".to_string(),
        ],
        authority: AgentAuthority::default(),
        lifecycle: AgentLifecycle::ready(),
        budget: AgentBudget::default(),
    })
}

pub fn normalize_root_manifest(mut manifest: AgentManifest) -> AgentManifest {
    manifest.authority.accessible_roots.clear();
    manifest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_manifest_is_unscoped_for_the_root_worker() {
        let manifest = root_manifest().unwrap_or_else(|error| panic!("root manifest: {error}"));
        assert!(manifest.authority.accessible_roots.is_empty());
    }

    #[test]
    fn normalize_root_manifest_clears_old_scoped_roots() {
        let mut manifest = root_manifest().unwrap_or_else(|error| panic!("root manifest: {error}"));
        manifest
            .authority
            .accessible_roots
            .push(PathBuf::from("/tmp"));
        let normalized = normalize_root_manifest(manifest);
        assert!(normalized.authority.accessible_roots.is_empty());
    }
}
