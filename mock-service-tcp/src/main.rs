use clap::Parser;
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

#[derive(Parser, Debug)]
#[command(
    name = "mock-service-tcp",
    about = "TCP echo server for rs-rok integration tests"
)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value_t = 9998)]
    port: u16,

    /// Mode: "echo" returns data as-is, "prefix" prepends "ECHO: " to each line
    #[arg(short, long, default_value = "echo")]
    mode: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = TcpListener::bind(addr).await.expect("failed to bind");

    info!("mock-service-tcp listening on {} (mode: {})", addr, args.mode);

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("accept error: {e}");
                continue;
            }
        };

        let mode = args.mode.clone();
        tokio::spawn(async move {
            info!("connection from {peer}");
            if let Err(e) = handle_connection(stream, &mode).await {
                debug!("connection {peer} ended: {e}");
            } else {
                debug!("connection {peer} closed");
            }
        });
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    mode: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    match mode {
        "prefix" => {
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                let response = format!("ECHO: {line}");
                writer.write_all(response.as_bytes()).await?;
                writer.flush().await?;
            }
        }
        _ => {
            // Raw echo: read chunks and write them back
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = tokio::io::AsyncReadExt::read(&mut reader, &mut buf).await?;
                if n == 0 {
                    break;
                }
                writer.write_all(&buf[..n]).await?;
                writer.flush().await?;
            }
        }
    }

    Ok(())
}
