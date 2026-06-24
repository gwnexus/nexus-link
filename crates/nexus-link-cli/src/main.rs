use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod commands;

#[derive(Parser)]
#[command(
    name = "nexus-link",
    about = "Nexus hardware node management CLI",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to config file
    #[arg(long, global = true)]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Register this node with the Nexus backend
    Register {
        /// Nexus API base URL
        #[arg(long, default_value = "https://nexus.gatewarden.eu")]
        api_url: String,

        /// Node registration token (nxs_node_*)
        #[arg(long)]
        token: String,

        /// C&C command token (nxs_cmd_*) for compose management.
        /// Generated in the Nexus dashboard alongside the node token.
        #[arg(long)]
        cmd_token: Option<String>,

        /// Human-readable name for this node
        #[arg(long)]
        name: Option<String>,

        /// Tags for categorization
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,

        /// Skip the device preflight check
        #[arg(long)]
        skip_preflight: bool,

        /// Force registration even if device is not recommended
        #[arg(long)]
        force: bool,
    },

    /// Run device preflight check without registering
    Preflight,

    /// Refresh the node token (token rotation)
    Refresh {
        /// New node token from the Nexus dashboard
        #[arg(long)]
        token: String,
    },

    /// Show current node status
    Status,

    /// Unregister this node and remove local credentials
    Unregister {
        /// Skip confirmation prompt and suppress errors
        #[arg(long)]
        force: bool,
    },

    /// Upgrade nexus-link to the latest release
    Upgrade {
        /// Force re-download even if already on latest version
        #[arg(long)]
        force: bool,
    },

    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Agent daemon management
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Display current configuration
    Show,
    /// Set a configuration value (e.g. nexus-link config set api_url https://...)
    Set {
        /// Config key (api_url, push_interval, listen_addr, port, name, tags)
        key: String,
        /// New value
        value: String,
    },
    /// Get a configuration value
    Get {
        /// Config key
        key: String,
    },
    /// Show the config file path
    Path,
}

#[derive(Subcommand)]
enum AgentAction {
    /// Start the telemetry agent and command service
    Start,
    /// Stop the running agent and service
    Stop,
    /// Show agent logs
    Logs {
        /// Number of tail lines
        #[arg(short, long, default_value = "50")]
        tail: u32,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Register {
            api_url,
            token,
            cmd_token,
            name,
            tags,
            skip_preflight,
            force,
        } => commands::register::execute(api_url, token, cmd_token, name, tags, skip_preflight, force).await,
        Commands::Preflight => {
            let report = nexus_link_core::preflight::run_preflight();
            nexus_link_core::preflight::print_report(&report);
            match report.verdict {
                nexus_link_core::preflight::PreflightVerdict::Incompatible => {
                    std::process::exit(1);
                }
                _ => Ok(()),
            }
        }
        Commands::Refresh { token } => commands::refresh::execute(token).await,
        Commands::Status => commands::status::execute().await,
        Commands::Unregister { force } => commands::unregister::execute(force).await,
        Commands::Upgrade { force } => commands::upgrade::execute(force).await,
        Commands::Config { action } => match action {
            ConfigAction::Show => commands::config::show().await,
            ConfigAction::Set { key, value } => commands::config::set(key, value).await,
            ConfigAction::Get { key } => commands::config::get(key).await,
            ConfigAction::Path => commands::config::path().await,
        },
        Commands::Agent { action } => match action {
            AgentAction::Start => commands::agent::start().await,
            AgentAction::Stop => commands::agent::stop().await,
            AgentAction::Logs { tail } => commands::agent::logs(tail).await,
        },
    }
}
