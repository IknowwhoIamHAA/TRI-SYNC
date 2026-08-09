#[derive(clap::Subcommand)]
pub enum Commands {
    Run {
        #[arg(short, long)]
        workflow: PathBuf,

        #[arg(short, long)]
        log: PathBuf,
    },
    Replay {
        #[arg(short, long)]
        log: PathBuf,
    }
}

pub fn dispatch(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Run { workflow, log } => run_workflow(workflow, log),
        Commands::Replay { log } => replay_log(log),
    }
}
