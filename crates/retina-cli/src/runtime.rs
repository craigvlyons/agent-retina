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
    let manifest = AgentManifest {
        agent_id: AgentId("root".to_string()),
        domain: "orchestrator".to_string(),
        status: AgentStatus::Idle,
        description: "Retina root agent running in independent v1 mode.".to_string(),
        created_at: Utc::now(),
    };
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
