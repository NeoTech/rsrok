use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "rs-rok", version, about = "Expose local services to the internet")]
pub struct Cli {
    /// Path to config file (default: ~/.rs-rok/settings.json)
    #[arg(long = "config", global = true)]
    pub config_path: Option<String>,

    /// Log level: trace, debug, info, warn, error
    #[arg(long = "log", global = true, default_value = "info")]
    pub log_level: String,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Expose a local HTTP service
    Http {
        /// Local port to forward traffic to
        port: u16,

        /// Stable tunnel name (e.g. --name myapp → /tunnel/myapp)
        #[arg(long)]
        name: Option<String>,

        /// Local hostname to forward to
        #[arg(long, default_value = "localhost")]
        host: String,
    },

    /// Expose a local HTTPS service (TLS terminated at edge)
    Https {
        /// Local port to forward traffic to
        port: u16,

        /// Stable tunnel name (e.g. --name myapp → /tunnel/myapp)
        #[arg(long)]
        name: Option<String>,

        /// Local hostname to forward to
        #[arg(long, default_value = "localhost")]
        host: String,
    },

    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Deploy the Cloudflare Worker to your account
    Deploy {
        /// Cloudflare Account ID (or CF_ACCOUNT_ID env var)
        #[arg(long)]
        account_id: Option<String>,

        /// Cloudflare API Token with Workers:Edit permission (or CF_API_TOKEN env var)
        #[arg(long)]
        api_token: Option<String>,

        /// Worker script name
        #[arg(long, default_value = "rs-rok")]
        name: String,
    },

    /// Expose a local TCP service (e.g. SSH, database)
    Tcp {
        /// Local port to forward TCP traffic to
        port: u16,

        /// Stable tunnel name (required for TCP)
        #[arg(long)]
        name: Option<String>,

        /// Local hostname to forward to
        #[arg(long, default_value = "localhost")]
        host: String,
    },

    /// Connect to a remote TCP tunnel
    Connect {
        /// Tunnel slug to connect to
        slug: String,

        /// Shared secret token from the server
        #[arg(long)]
        token: String,

        /// Local port to listen on
        #[arg(long, default_value_t = 0)]
        port: u16,

        /// Local address to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Store an authentication token
    AddToken {
        /// The auth token to store
        token: String,
    },

    /// Print current configuration
    Show,

    /// Set the worker endpoint URL
    SetEndpoint {
        /// The worker endpoint URL
        url: String,
    },

    /// Store Cloudflare credentials for deploy
    SetCfCredentials {
        /// Cloudflare Account ID
        #[arg(long)]
        account_id: String,

        /// Cloudflare API Token with Workers:Edit permission
        #[arg(long)]
        api_token: String,
    },
}
