mod app;
mod chat;
mod cli;
mod controller;
mod output;
mod runtime;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{app, cli, runtime};
    use retina_memory_sqlite::SqliteMemory;
    use retina_traits::Memory;
    use std::env;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn init_creates_runtime_layout() {
        let _guard = env_lock().lock().unwrap();
        let dir = tempdir().unwrap();
        unsafe {
            env::set_var("RETINA_HOME", dir.path());
        }
        runtime::init_runtime().unwrap();
        assert!(dir.path().join("config.toml").exists());
        assert!(dir.path().join("root").join("agent.db").exists());
        assert!(dir.path().join("root").join("manifest.toml").exists());
    }

    #[test]
    fn run_creates_timeline_entries() {
        let _guard = env_lock().lock().unwrap();
        let dir = tempdir().unwrap();
        unsafe {
            env::set_var("RETINA_HOME", dir.path());
        }
        app::run_task("inspect working directory".to_string()).unwrap();
        let memory = SqliteMemory::open(dir.path().join("root").join("agent.db")).unwrap();
        assert!(!memory.recent_states(10).unwrap().is_empty());
    }

    #[test]
    fn cli_help_includes_chat_command_shape() {
        use clap::CommandFactory;
        let command = cli::Cli::command();
        let subcommands = command
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_string())
            .collect::<Vec<_>>();
        assert!(subcommands.contains(&"chat".to_string()));
    }
}
