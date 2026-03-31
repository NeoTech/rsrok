use futures_util::{SinkExt, StreamExt};
use rs_rok_protocol::{decode, encode, Frame};
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::connect_async;
use tracing::{debug, error, info, warn};

static STREAM_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_stream_id() -> u32 {
    STREAM_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn next_request_id() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub struct TcpClientConfig {
    pub endpoint: String,
    pub slug: String,
    pub token: String,
    pub local_addr: String,
}

pub async fn run(
    config: TcpClientConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(&config.local_addr).await?;
    let actual_addr = listener.local_addr()?;

    println!();
    println!("rs-rok connect                        (Ctrl+C to quit)");
    println!();
    println!("Listening:  {actual_addr}");
    println!("Tunnel:     {}", config.slug);
    println!();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl+c");
        info!("shutdown signal received");
        let _ = shutdown_tx.send(true);
    });

    loop {
        let mut shutdown_for_accept = shutdown_rx.clone();
        let tcp_stream = tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        debug!("accepted TCP connection from {addr}");
                        stream
                    }
                    Err(e) => {
                        error!("accept error: {e}");
                        continue;
                    }
                }
            }
            _ = shutdown_for_accept.changed() => {
                info!("shutting down TCP client");
                return Ok(());
            }
        };

        let endpoint = config.endpoint.clone();
        let slug = config.slug.clone();
        let token = config.token.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_tcp_connection(tcp_stream, &endpoint, &slug, &token).await {
                warn!("TCP relay error: {e}");
            }
        });
    }
}

async fn handle_tcp_connection(
    tcp_stream: tokio::net::TcpStream,
    endpoint: &str,
    slug: &str,
    token: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stream_id = next_stream_id();

    // Build WebSocket URL with dedicated TCP path prefix
    let ws_base = endpoint
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let ws_url = format!("{ws_base}/__rsrok_tcp__/{slug}");

    let req = ws_url.into_client_request()?;

    let (ws_stream, _) = connect_async(req).await?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Send TCP_OPEN
    let open_frame = Frame::TcpOpen {
        request_id: next_request_id(),
        stream_id,
        token: token.to_string(),
    };
    ws_write.send(Message::Binary(encode(&open_frame).into())).await?;

    // Wait for TCP_OPEN_ACK or ERROR
    let ack_timeout = tokio::time::Duration::from_secs(30);
    let ack = tokio::time::timeout(ack_timeout, async {
        while let Some(msg) = ws_read.next().await {
            match msg {
                Ok(Message::Binary(data)) => {
                    match decode(&data) {
                        Ok((Frame::TcpOpenAck { stream_id: sid, .. }, _)) if sid == stream_id => {
                            return Ok(true);
                        }
                        Ok((Frame::Error { code, message, .. }, _)) => {
                            return Err(format!("server rejected connection ({code}): {message}"));
                        }
                        Ok((Frame::TcpClose { reason, .. }, _)) => {
                            return Err(format!("server closed stream: {reason}"));
                        }
                        _ => continue,
                    }
                }
                Ok(Message::Close(_)) | Err(_) => {
                    return Err("WebSocket closed before ACK".to_string());
                }
                _ => continue,
            }
        }
        Err("WebSocket stream ended before ACK".to_string())
    })
    .await
    .map_err(|_| "TCP_OPEN_ACK timed out (30s)")??;

    if !ack {
        return Err("did not receive TCP_OPEN_ACK".into());
    }

    debug!(stream_id, "TCP stream opened, starting relay");

    // Bidirectional relay
    let (mut tcp_read, mut tcp_write) = tcp_stream.into_split();
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        tokio::select! {
            result = tcp_read.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        // TCP EOF — close the stream
                        let close_frame = Frame::TcpClose {
                            request_id: next_request_id(),
                            stream_id,
                            reason: String::new(),
                        };
                        let _ = ws_write.send(Message::Binary(encode(&close_frame).into())).await;
                        break;
                    }
                    Ok(n) => {
                        let data_frame = Frame::TcpData {
                            request_id: next_request_id(),
                            stream_id,
                            data: buf[..n].to_vec(),
                        };
                        if ws_write.send(Message::Binary(encode(&data_frame).into())).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let close_frame = Frame::TcpClose {
                            request_id: next_request_id(),
                            stream_id,
                            reason: format!("local read error: {e}"),
                        };
                        let _ = ws_write.send(Message::Binary(encode(&close_frame).into())).await;
                        break;
                    }
                }
            }
            msg = ws_read.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        match decode(&data) {
                            Ok((Frame::TcpData { data: payload, .. }, _)) => {
                                if let Err(e) = tcp_write.write_all(&payload).await {
                                    debug!(stream_id, "local TCP write error: {e}");
                                    break;
                                }
                            }
                            Ok((Frame::TcpClose { reason, .. }, _)) => {
                                debug!(stream_id, "remote closed stream: {reason}");
                                break;
                            }
                            Ok((Frame::Error { code, message, .. }, _)) => {
                                warn!(stream_id, "error from server ({code}): {message}");
                                break;
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        debug!(stream_id, "WebSocket closed");
                        break;
                    }
                    Some(Err(e)) => {
                        debug!(stream_id, "WebSocket error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = tcp_write.shutdown().await;
    let _ = ws_write.send(Message::Close(None)).await;
    Ok(())
}
