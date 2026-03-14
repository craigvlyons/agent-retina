use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "retina")]
#[command(about = "Retina v1 CLI agent")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Init,
    Run {
        task: String,
    },
    Cleanup {
        #[arg(long, default_value_t = 2000)]
        keep_events: usize,
        #[arg(long, default_value_t = 30)]
        stale_knowledge_days: u64,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        optimize: bool,
    },
    Chat,
    Inspect {
        #[command(subcommand)]
        command: InspectCommands,
    },
    Stats,
}

#[derive(Subcommand)]
pub enum InspectCommands {
    Timeline,
    Overview,
    Agents,
    Memory {
        #[arg(default_value = "")]
        query: String,
    },
}
