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
    Agents,
    Memory {
        #[arg(default_value = "")]
        query: String,
    },
}
