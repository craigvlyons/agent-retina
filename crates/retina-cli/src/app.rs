use crate::chat::ChatSession;
use crate::cli::{Cli, Commands, InspectCommands};
use crate::controller::{AgentController, InspectController};
use crate::maintenance::run_cleanup;
use crate::output::{
    render_action_result, render_agent_registry, render_cleanup_report, render_memory_inspection,
    render_stats, render_task_state, render_timeline, render_worker_overview,
};
use crate::runtime::init_runtime;
use clap::Parser;
use retina_types::*;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => init_runtime(),
        Commands::Run { task } => run_task(task),
        Commands::Cleanup {
            keep_events,
            stale_knowledge_days,
            optimize,
        } => cleanup(keep_events, stale_knowledge_days, optimize),
        Commands::Chat => chat(),
        Commands::Inspect { command } => inspect(command),
        Commands::Stats => stats(),
    }
}

pub fn run_task(task: String) -> Result<()> {
    init_runtime()?;
    let controller = AgentController::new(false)?;
    let outcome = controller.execute_task(task)?;
    match outcome {
        Outcome::Success(result) => println!("{}", render_action_result(&result)),
        Outcome::Failure(reason) => println!("Task failed: {reason}"),
        Outcome::Blocked(reason) => println!("Task blocked: {reason}"),
    }
    Ok(())
}

pub fn chat() -> Result<()> {
    init_runtime()?;
    let mut session = ChatSession::new()?;
    session.run()
}

pub fn cleanup(keep_events: usize, stale_knowledge_days: u64, optimize: bool) -> Result<()> {
    let report = run_cleanup(keep_events, stale_knowledge_days, optimize)?;
    print!(
        "{}",
        render_cleanup_report(&report, keep_events, stale_knowledge_days, optimize)
    );
    Ok(())
}

pub fn inspect(command: InspectCommands) -> Result<()> {
    let inspector = InspectController::new()?;
    match command {
        InspectCommands::Timeline => {
            let events = inspector.recent_timeline(50)?;
            print!("{}", render_timeline(&events));
        }
        InspectCommands::TaskState => match inspector.latest_task_state()? {
            Some(task_state) => print!("{}", render_task_state(&task_state)),
            None => println!("No task state snapshots recorded yet."),
        },
        InspectCommands::Overview => {
            let overview = inspector.worker_overview()?;
            print!("{}", render_worker_overview(&overview));
        }
        InspectCommands::Agents => {
            let registry = inspector.agent_registry()?;
            print!("{}", render_agent_registry(&registry));
        }
        InspectCommands::Memory { query } => {
            let (knowledge, experiences) = inspector.memory_lookup(&query, 10)?;
            print!("{}", render_memory_inspection(&knowledge, &experiences));
        }
    }
    Ok(())
}

pub fn stats() -> Result<()> {
    let inspector = InspectController::new()?;
    let stats = inspector.stats()?;
    print!("{}", render_stats(&stats));
    Ok(())
}
