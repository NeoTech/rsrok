mod cli;
mod cloudflare_config;
mod config;
mod deploy;
mod proxy;
mod tcp_client;
mod tunnel;
mod worker_bundle;

use clap::Parser;
use cli::{Cli, Command, ConfigAction};
use cloudflare_config::CloudflareConfig;
use config::Settings;
use rs_rok_protocol::TunnelType;
use tracing::error;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Init tracing
    let env_filter = tracing_subscriber::EnvFilter::try_new(&cli.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let config_path = Settings::config_path(cli.config_path.as_deref());

    match cli.command {
        Command::Config { action } => {
            let mut settings = Settings::load(&config_path);
            match action {
                ConfigAction::AddToken { token } => {
                    settings.auth_token = Some(token);
                    if let Err(e) = settings.save(&config_path) {
                        error!("failed to save config: {e}");
                        std::process::exit(1);
                    }
                    println!("Token saved to {}", config_path.display());
                }
                ConfigAction::Show => {
                    let json = serde_json::to_string_pretty(&settings)
                        .expect("failed to serialize settings");
                    println!("{json}");
                }
                ConfigAction::SetEndpoint { url } => {
                    settings.endpoint = url;
                    if let Err(e) = settings.save(&config_path) {
                        error!("failed to save config: {e}");
                        std::process::exit(1);
                    }
                    println!("Endpoint saved to {}", config_path.display());
                }
                ConfigAction::SetCfCredentials {
                    account_id,
                    api_token,
                } => {
                    let cf_path = CloudflareConfig::config_path();
                    let cf = CloudflareConfig {
                        account_id,
                        api_token,
                    };
                    if let Err(e) = cf.save(&cf_path) {
                        error!("failed to save cloudflare config: {e}");
                        std::process::exit(1);
                    }
                    println!("Cloudflare credentials saved to {}", cf_path.display());
                }
            }
        }
        Command::Http {
            port, host, name,
        } => {
            start_tunnel(TunnelType::Http, port, &host, name, &config_path).await;
        }
        Command::Https {
            port, host, name,
        } => {
            start_tunnel(TunnelType::Https, port, &host, name, &config_path).await;
        }
        Command::Deploy {
            account_id,
            api_token,
            name,
        } => {
            deploy_worker(account_id, api_token, &name, &config_path).await;
        }
        Command::Tcp {
            port, host, name,
        } => {
            start_tcp_tunnel(port, &host, name, &config_path).await;
        }
        Command::Connect {
            slug, token, port, host,
        } => {
            start_tcp_client(&slug, &token, port, &host, &config_path).await;
        }
    }
}

async fn start_tunnel(
    tunnel_type: TunnelType,
    port: u16,
    host: &str,
    name: Option<String>,
    config_path: &std::path::Path,
) {
    let settings = Settings::load(config_path);
    let auth_token = settings.auth_token.unwrap_or_default();
    let local_addr = format!("{host}:{port}");

    let tunnel_config = tunnel::TunnelConfig {
        endpoint: settings.endpoint,
        auth_token,
        tunnel_type,
        local_addr,
        name,
        tcp_token: None,
    };

    if let Err(e) = tunnel::run(tunnel_config).await {
        error!("tunnel error: {e}");
        std::process::exit(1);
    }
}

async fn start_tcp_tunnel(
    port: u16,
    host: &str,
    name: Option<String>,
    config_path: &std::path::Path,
) {
    use rand::Rng;

    let settings = Settings::load(config_path);
    let auth_token = settings.auth_token.unwrap_or_default();
    let local_addr = format!("{host}:{port}");

    // Generate a random 32-char hex token
    let mut rng = rand::thread_rng();
    let token_bytes: [u8; 16] = rng.gen();
    let tcp_token: String = token_bytes.iter().map(|b| format!("{b:02x}")).collect();

    let slug = name.as_deref().unwrap_or("__root__");
    println!();
    println!("TCP tunnel token: {tcp_token}");
    println!("Connect with:     rs-rok connect {slug} --token {tcp_token} --port <local-port>");
    println!();

    let tunnel_config = tunnel::TunnelConfig {
        endpoint: settings.endpoint,
        auth_token,
        tunnel_type: TunnelType::Tcp,
        local_addr,
        name,
        tcp_token: Some(tcp_token),
    };

    if let Err(e) = tunnel::run(tunnel_config).await {
        error!("tunnel error: {e}");
        std::process::exit(1);
    }
}

async fn start_tcp_client(
    slug: &str,
    token: &str,
    port: u16,
    host: &str,
    config_path: &std::path::Path,
) {
    let settings = Settings::load(config_path);

    let client_config = tcp_client::TcpClientConfig {
        endpoint: settings.endpoint,
        slug: slug.to_string(),
        token: token.to_string(),
        local_addr: format!("{host}:{port}"),
    };

    if let Err(e) = tcp_client::run(client_config).await {
        error!("connect error: {e}");
        std::process::exit(1);
    }
}

async fn deploy_worker(
    account_id: Option<String>,
    api_token: Option<String>,
    worker_name: &str,
    config_path: &std::path::Path,
) {
    let cf_path = CloudflareConfig::config_path();
    let mut cf = CloudflareConfig::load(&cf_path).unwrap_or(CloudflareConfig {
        account_id: String::new(),
        api_token: String::new(),
    });

    // CLI flags override stored/env config
    if let Some(id) = account_id {
        cf.account_id = id;
    }
    if let Some(tok) = api_token {
        cf.api_token = tok;
    }

    if cf.account_id.is_empty() {
        error!("Cloudflare Account ID required. Provide --account-id or set CF_ACCOUNT_ID, or run: rs-rok config set-cf-credentials");
        std::process::exit(1);
    }
    if cf.api_token.is_empty() {
        error!("Cloudflare API Token required. Provide --api-token or set CF_API_TOKEN, or run: rs-rok config set-cf-credentials");
        std::process::exit(1);
    }

    println!("Deploying worker '{worker_name}'...");

    match deploy::deploy_worker(&cf, worker_name).await {
        Ok(url) => {
            println!("Worker deployed successfully!");
            println!("URL: {url}");

            // Save credentials for future deploys
            if let Err(e) = cf.save(&cf_path) {
                error!("warning: could not save credentials: {e}");
            }

            // Update endpoint in settings
            let mut settings = Settings::load(config_path);
            settings.endpoint = url.clone();
            if let Err(e) = settings.save(config_path) {
                error!("warning: could not update endpoint in settings: {e}");
            } else {
                println!("Endpoint updated to {url}");
            }
        }
        Err(e) => {
            error!("deploy failed: {e}");
            std::process::exit(1);
        }
    }
}
