mod cli;
mod cloudflare_config;
mod config;
mod deploy;
mod proxy;
mod saved_tunnels;
mod tcp_client;
mod tui;
mod tunnel;
mod worker_bundle;

use clap::Parser;
use cli::{Cli, Command, ConfigAction};
use cloudflare_config::CloudflareConfig;
use config::Settings;
use rs_rok_protocol::TunnelType;
use std::io::IsTerminal;
use tracing::error;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let config_path = Settings::config_path(cli.config_path.as_deref());

    // No subcommand + interactive TTY -> launch TUI
    if cli.command.is_none() && std::io::stdout().is_terminal() {
        // On Windows, detect raw mintty (pipe handle) by probing the actual console handle.
        // MSYSTEM is always set in Git Bash/MSYS2 regardless of whether ConPTY is active,
        // so env var checks alone are not enough. crossterm::terminal::size() calls
        // GetConsoleScreenBufferInfo under the hood — it succeeds when ConPTY is present
        // (winpty, MSYS=enable_pcon, VS Code, Windows Terminal) and fails on a raw pipe.
        #[cfg(windows)]
        if crossterm::terminal::size().is_err() {
            eprintln!("rsrok TUI does not work in this terminal (raw pipe handle detected).");
            eprintln!();
            eprintln!("Options:");
            eprintln!("  1. Run in Windows Terminal, PowerShell, or Command Prompt (recommended)");
            eprintln!("  2. Prefix with winpty (ships with Git for Windows):");
            eprintln!("       winpty rsrok");
            eprintln!("  3. Enable ConPTY in mintty by adding to ~/.bashrc:");
            eprintln!("       export MSYS=enable_pcon");
            eprintln!("     then restart Git Bash (requires mintty >= 3.0)");
            eprintln!();
            eprintln!("Note: CLI subcommands still work from any terminal: rsrok http, rsrok tcp, ...");
            std::process::exit(1);
        }

        if let Err(e) = tui::run(config_path, cli.profile).await {
            eprintln!("TUI error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Init tracing (only for CLI mode — TUI handles its own output)
    let env_filter = tracing_subscriber::EnvFilter::try_new(&cli.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let Some(command) = cli.command else {
        // Not a TTY and no subcommand — print help
        use clap::CommandFactory;
        Cli::command().print_help().ok();
        std::process::exit(0);
    };

    match command {
        Command::Config { action } => {
            let mut settings = Settings::load(&config_path);
            if let Some(ref name) = cli.profile {
                if !settings.switch_active_by_name(name) {
                    error!("profile '{}' not found", name);
                    std::process::exit(1);
                }
            }
            match action {
                ConfigAction::AddToken { token } => {
                    settings.active_profile_mut().auth_token = Some(token);
                    if let Err(e) = settings.save(&config_path) {
                        error!("failed to save config: {e}");
                        std::process::exit(1);
                    }
                    println!("Token saved to {}", config_path.display());
                }
                ConfigAction::Show => {
                    let json = serde_json::to_string_pretty(&settings.profiles)
                        .expect("failed to serialize settings");
                    println!("{json}");
                }
                ConfigAction::SetEndpoint { url } => {
                    settings.active_profile_mut().endpoint = url;
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
                    let mut cfg = CloudflareConfig::load(&cf_path);
                    cfg.accounts.push(crate::cloudflare_config::CfAccount {
                        name: String::new(),
                        account_id,
                        api_token,
                    });
                    if let Err(e) = cfg.save(&cf_path) {
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
            start_tunnel(TunnelType::Http, port, &host, name, cli.profile.as_deref(), &config_path).await;
        }
        Command::Https {
            port, host, name,
        } => {
            start_tunnel(TunnelType::Https, port, &host, name, cli.profile.as_deref(), &config_path).await;
        }
        Command::Deploy {
            account_id,
            api_token,
            auth_token,
            name,
        } => {
            deploy_worker(account_id, api_token, auth_token, &name, &config_path).await;
        }
        Command::Tcp {
            port, host, name,
        } => {
            start_tcp_tunnel(port, &host, name, cli.profile.as_deref(), &config_path).await;
        }
        Command::Connect {
            slug, token, port, host,
        } => {
            start_tcp_client(&slug, &token, port, &host, cli.profile.as_deref(), &config_path).await;
        }
    }
}

/// If --profile was passed, switch to that profile (or exit on unknown name).
fn apply_profile_flag(settings: &mut Settings, profile_name: Option<&str>) {
    if let Some(name) = profile_name {
        if !settings.switch_active_by_name(name) {
            error!("profile '{}' not found", name);
            std::process::exit(1);
        }
    }
}

/// Load settings from disk, apply the --profile flag, and return the active profile.
fn load_active_profile(config_path: &std::path::Path, profile_name: Option<&str>) -> config::Profile {
    let mut settings = Settings::load(config_path);
    apply_profile_flag(&mut settings, profile_name);
    settings.active_profile().clone()
}

async fn start_tunnel(
    tunnel_type: TunnelType,
    port: u16,
    host: &str,
    name: Option<String>,
    profile: Option<&str>,
    config_path: &std::path::Path,
) {
    let active = load_active_profile(config_path, profile);
    let local_addr = format!("{host}:{port}");

    let tunnel_config = tunnel::TunnelConfig {
        endpoint: active.endpoint.clone(),
        auth_token: active.auth_token.clone().unwrap_or_default(),
        tunnel_type,
        local_addr,
        name,
        tcp_token: None,
        events_tx: None,
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
    profile: Option<&str>,
    config_path: &std::path::Path,
) {
    use rand::Rng;

    let active = load_active_profile(config_path, profile);
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
        endpoint: active.endpoint.clone(),
        auth_token: active.auth_token.clone().unwrap_or_default(),
        tunnel_type: TunnelType::Tcp,
        local_addr,
        name,
        tcp_token: Some(tcp_token),
        events_tx: None,
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
    profile: Option<&str>,
    config_path: &std::path::Path,
) {
    let active = load_active_profile(config_path, profile);

    let client_config = tcp_client::TcpClientConfig {
        endpoint: active.endpoint.clone(),
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
    auth_token: Option<String>,
    worker_name: &str,
    config_path: &std::path::Path,
) {
    let cf_path = CloudflareConfig::config_path();
    let mut cf_cfg = CloudflareConfig::load(&cf_path);
    let mut cf = cf_cfg.first().cloned().unwrap_or(crate::cloudflare_config::CfAccount {
        name: String::new(),
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

    // Load settings once — used both to resolve auth_token fallback and to save result.
    let mut settings = Settings::load(config_path);

    // Use CLI flag if given, otherwise fall back to active profile's auth_token.
    let effective_auth_token = if auth_token.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
        auth_token
    } else {
        settings.active_profile().auth_token.clone()
    };

    if cf.account_id.is_empty() {
        error!("Cloudflare Account ID required. Provide --account-id or set CF_ACCOUNT_ID, or run: rs-rok config set-cf-credentials");
        std::process::exit(1);
    }
    if cf.api_token.is_empty() {
        error!("Cloudflare API Token required. Provide --api-token or set CF_API_TOKEN, or run: rs-rok config set-cf-credentials");
        std::process::exit(1);
    }

    println!("Deploying worker '{worker_name}'...");

    match deploy::deploy_worker(&cf, worker_name, effective_auth_token.as_deref()).await {
        Ok(url) => {
            println!("Worker deployed successfully!");
            println!("URL: {url}");

            // Persist CF credentials via shared upsert (no duplicate add).
            cf_cfg.upsert_account(cf);
            if let Err(e) = cf_cfg.save(&cf_path) {
                error!("warning: could not save credentials: {e}");
            }

            // Apply deploy result to active profile via shared method, then save.
            settings.apply_deploy_result(&url, effective_auth_token.as_deref());
            if let Err(e) = settings.save(config_path) {
                error!("warning: could not update settings: {e}");
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
