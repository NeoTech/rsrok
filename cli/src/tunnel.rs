use futures_util::{SinkExt, StreamExt};
use rs_rok_protocol::Header;
use rs_rok_protocol::{decode, encode, Frame, TunnelType};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::connect_async;
use tracing::{debug, error, info, warn};

use crate::proxy;
use crate::proxy::ForwardScheme;

static REQUEST_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_request_id() -> u32 {
    REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug)]
enum LocalWsCommand {
    Data { is_binary: bool, data: Vec<u8> },
    Close { code: u16, reason: String },
}

fn spawn_local_ws_bridge(
    ws_id: u32,
    local_addr: String,
    url: String,
    headers: Vec<Header>,
    protocols: Vec<String>,
    scheme: ForwardScheme,
    out_tx: mpsc::UnboundedSender<Frame>,
) -> mpsc::UnboundedSender<LocalWsCommand> {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<LocalWsCommand>();

    tokio::spawn(async move {
        let ws_scheme = match scheme {
            ForwardScheme::Http => "ws",
            ForwardScheme::Https => "wss",
        };
        let ws_target = format!("{ws_scheme}://{local_addr}{url}");
        let mut req = match ws_target.clone().into_client_request() {
            Ok(req) => req,
            Err(e) => {
                let _ = out_tx.send(Frame::WsClose {
                    request_id: next_request_id(),
                    ws_id,
                    code: 1011,
                    reason: format!("invalid websocket target: {e}"),
                });
                return;
            }
        };

        let hop_by_hop = [
            "connection",
            "keep-alive",
            "transfer-encoding",
            "upgrade",
            "host",
            "sec-websocket-key",
            "sec-websocket-version",
            "sec-websocket-extensions",
            "sec-websocket-protocol",
        ];
        for h in headers {
            if hop_by_hop
                .iter()
                .any(|name| h.name.eq_ignore_ascii_case(name))
            {
                continue;
            }

            let Ok(name) = tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(
                h.name.as_bytes(),
            ) else {
                continue;
            };
            let Ok(value) = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&h.value)
            else {
                continue;
            };
            req.headers_mut().append(name, value);
        }

        if !protocols.is_empty() {
            let joined = protocols.join(", ");
            if let Ok(value) =
                tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&joined)
            {
                req.headers_mut().insert(
                    tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL,
                    value,
                );
            }
        }

        let (local_ws, _) = match connect_async(req).await {
            Ok(v) => v,
            Err(e) => {
                let _ = out_tx.send(Frame::WsClose {
                    request_id: next_request_id(),
                    ws_id,
                    code: 1011,
                    reason: format!("local websocket connect failed: {e}"),
                });
                return;
            }
        };

        let (mut local_write, mut local_read) = local_ws.split();

        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(LocalWsCommand::Data { is_binary, data }) => {
                            let outbound = if is_binary {
                                Message::Binary(data.into())
                            } else {
                                match String::from_utf8(data) {
                                    Ok(text) => Message::Text(text.into()),
                                    Err(e) => Message::Binary(e.into_bytes().into()),
                                }
                            };
                            if let Err(e) = local_write.send(outbound).await {
                                let _ = out_tx.send(Frame::WsClose {
                                    request_id: next_request_id(),
                                    ws_id,
                                    code: 1011,
                                    reason: format!("local websocket write failed: {e}"),
                                });
                                break;
                            }
                        }
                        Some(LocalWsCommand::Close { code, reason }) => {
                            debug!(ws_id, code, reason = %reason, "closing local websocket session");
                            let _ = local_write.send(Message::Close(None)).await;
                            break;
                        }
                        None => {
                            let _ = local_write.send(Message::Close(None)).await;
                            break;
                        }
                    }
                }
                msg = local_read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            let _ = out_tx.send(Frame::WsData {
                                request_id: next_request_id(),
                                ws_id,
                                is_binary: false,
                                data: text.to_string().into_bytes(),
                            });
                        }
                        Some(Ok(Message::Binary(data))) => {
                            let _ = out_tx.send(Frame::WsData {
                                request_id: next_request_id(),
                                ws_id,
                                is_binary: true,
                                data: data.to_vec(),
                            });
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let (code, reason) = match frame {
                                Some(frame) => (u16::from(frame.code), frame.reason.to_string()),
                                None => (1000, String::new()),
                            };
                            let _ = out_tx.send(Frame::WsClose {
                                request_id: next_request_id(),
                                ws_id,
                                code,
                                reason,
                            });
                            break;
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = local_write.send(Message::Pong(payload)).await;
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            let _ = out_tx.send(Frame::WsClose {
                                request_id: next_request_id(),
                                ws_id,
                                code: 1011,
                                reason: format!("local websocket read failed: {e}"),
                            });
                            break;
                        }
                        None => {
                            let _ = out_tx.send(Frame::WsClose {
                                request_id: next_request_id(),
                                ws_id,
                                code: 1000,
                                reason: String::new(),
                            });
                            break;
                        }
                    }
                }
            }
        }
    });

    cmd_tx
}

pub struct TunnelConfig {
    pub endpoint: String,
    pub auth_token: String,
    pub tunnel_type: TunnelType,
    pub local_addr: String,
    pub name: Option<String>,
}

/// Run the tunnel with automatic reconnection via exponential backoff.
pub async fn run(config: TunnelConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl+c");
        info!("shutdown signal received");
        let _ = shutdown_tx.send(true);
    });

    let mut delay = Duration::from_secs(1);
    const MAX_DELAY: Duration = Duration::from_secs(30);

    loop {
        if *shutdown_rx.borrow() {
            return Ok(());
        }

        match connect_and_run(&config, shutdown_rx.clone()).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("409") {
                    eprintln!("error: {msg}");
                    return Err(e);
                }
                warn!("tunnel disconnected: {msg}, reconnecting...");
            }
        }

        // Interruptible sleep — exits immediately on Ctrl+C
        let mut shutdown_for_sleep = shutdown_rx.clone();
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = shutdown_for_sleep.changed() => return Ok(()),
        }
        delay = (delay * 2).min(MAX_DELAY);
    }
}

async fn connect_and_run(
    config: &TunnelConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tunnel_id = *uuid::Uuid::new_v4().as_bytes();

    let tunnel_slug = match &config.name {
        Some(name) => name.clone(),
        None => "__root__".to_string(),
    };

    let ws_base = config.endpoint
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let ws_url = format!("{}/__rsrok_cli__/{}", ws_base, tunnel_slug);
    info!("connecting to {ws_url}");

    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

    let (mut write, mut read) = ws_stream.split();

    // Build auth_token bytes: pad or truncate to 32 bytes
    let mut auth_bytes = [0u8; 32];
    let token_bytes = config.auth_token.as_bytes();
    let copy_len = token_bytes.len().min(32);
    auth_bytes[..copy_len].copy_from_slice(&token_bytes[..copy_len]);

    // Send REGISTER frame
    let register = Frame::Register {
        request_id: next_request_id(),
        tunnel_id,
        auth_token: auth_bytes,
        tunnel_type: config.tunnel_type,
    };
    write
        .send(Message::Binary(encode(&register).into()))
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

    debug!("REGISTER sent, waiting for ACK...");

    // Wait for REGISTER_ACK
    let public_url = loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        match decode(&data) {
                            Ok((Frame::RegisterAck { public_url, .. }, _)) => {
                                break public_url;
                            }
                            Ok((Frame::Error { message, code, .. }, _)) => {
                                return Err(format!("server error ({code}): {message}").into());
                            }
                            Ok(_) => {
                                debug!("ignoring unexpected frame while waiting for ACK");
                            }
                            Err(e) => {
                                return Err(format!("decode error: {e:?}").into());
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return Err("connection closed before REGISTER_ACK".into());
                    }
                    Some(Err(e)) => {
                        return Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>);
                    }
                    _ => {} // skip text/ping/pong WS-level frames
                }
            }
            _ = shutdown_rx.changed() => {
                return Ok(());
            }
        }
    };

    // Print status banner
    let local_scheme = match config.tunnel_type {
        TunnelType::Http => "http",
        TunnelType::Https => "https",
    };
    println!();
    println!("rs-rok                                (Ctrl+C to quit)");
    println!();
    println!("Tunnel:     {public_url}");
    println!("Forwarding: {local_scheme}://{}", config.local_addr);
    println!();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Frame>();
    let mut ws_sessions: HashMap<u32, mpsc::UnboundedSender<LocalWsCommand>> = HashMap::new();

    let writer_task = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            write.send(Message::Binary(encode(&frame).into())).await?;
        }
        let _ = write.send(Message::Close(None)).await;
        Ok::<(), tokio_tungstenite::tungstenite::Error>(())
    });

    // Main loop: handle incoming frames + heartbeat
    let heartbeat_interval = Duration::from_secs(25);
    let mut heartbeat = tokio::time::interval(heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Skip the first immediate tick
    heartbeat.tick().await;

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let frame = match decode(&data) {
                            Ok((frame, _)) => frame,
                            Err(e) => {
                                error!("failed to decode frame: {e:?}");
                                continue;
                            }
                        };

                        match frame {
                            Frame::Request { request_id, .. } => {
                                debug!(request_id, "received REQUEST, forwarding to local service");
                                let request_frame = frame;
                                let out_tx = out_tx.clone();
                                let local_addr = config.local_addr.clone();
                                let scheme = match config.tunnel_type {
                                    TunnelType::Http => ForwardScheme::Http,
                                    TunnelType::Https => ForwardScheme::Https,
                                };
                                tokio::spawn(async move {
                                    match proxy::forward_request(&request_frame, &local_addr, scheme, &out_tx).await {
                                        Ok(Some(resp)) => {
                                            let _ = out_tx.send(resp);
                                        }
                                        Ok(None) => {
                                            // Streaming response — frames already sent via out_tx
                                        }
                                        Err(e) => {
                                            warn!(request_id, "proxy error: {e}");
                                            let _ = out_tx.send(Frame::Error {
                                                request_id,
                                                code: 502,
                                                message: format!("proxy error: {e}"),
                                            });
                                        }
                                    }
                                });
                            }
                            Frame::WsOpen {
                                ws_id,
                                url,
                                headers,
                                protocols,
                                ..
                            } => {
                                if let Some(existing) = ws_sessions.remove(&ws_id) {
                                    let _ = existing.send(LocalWsCommand::Close {
                                        code: 1012,
                                        reason: "session replaced".into(),
                                    });
                                }
                                let ws_scheme = match config.tunnel_type {
                                    TunnelType::Http => ForwardScheme::Http,
                                    TunnelType::Https => ForwardScheme::Https,
                                };
                                let cmd_tx = spawn_local_ws_bridge(
                                    ws_id,
                                    config.local_addr.clone(),
                                    url,
                                    headers,
                                    protocols,
                                    ws_scheme,
                                    out_tx.clone(),
                                );
                                ws_sessions.insert(ws_id, cmd_tx);
                            }
                            Frame::WsData {
                                ws_id,
                                is_binary,
                                data,
                                ..
                            } => {
                                if let Some(cmd_tx) = ws_sessions.get(&ws_id) {
                                    if cmd_tx.send(LocalWsCommand::Data { is_binary, data }).is_err() {
                                        ws_sessions.remove(&ws_id);
                                    }
                                }
                            }
                            Frame::WsClose {
                                ws_id,
                                code,
                                reason,
                                ..
                            } => {
                                if let Some(cmd_tx) = ws_sessions.remove(&ws_id) {
                                    let _ = cmd_tx.send(LocalWsCommand::Close { code, reason });
                                }
                            }
                            Frame::Ping { request_id } => {
                                debug!("received PING, sending PONG");
                                let _ = out_tx.send(Frame::Pong { request_id });
                            }
                            Frame::Error {
                                code, message, ..
                            } => {
                                error!("received ERROR frame ({code}): {message}");
                            }
                            other => {
                                debug!("ignoring unexpected frame type: {:?}", other.frame_type());
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket closed by server");
                        for (_, tx) in ws_sessions.drain() {
                            let _ = tx.send(LocalWsCommand::Close {
                                code: 1012,
                                reason: "tunnel closed".into(),
                            });
                        }
                        drop(out_tx);
                        let _ = writer_task.await;
                        return Err("connection closed by server".into());
                    }
                    Some(Err(e)) => {
                        for (_, tx) in ws_sessions.drain() {
                            let _ = tx.send(LocalWsCommand::Close {
                                code: 1011,
                                reason: "tunnel error".into(),
                            });
                        }
                        drop(out_tx);
                        let _ = writer_task.await;
                        return Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>);
                    }
                    _ => {} // skip text/ping/pong WS-level frames
                }
            }
            _ = heartbeat.tick() => {
                let ping = Frame::Ping {
                    request_id: next_request_id(),
                };
                debug!("sending heartbeat PING");
                if out_tx.send(ping).is_err() {
                    for (_, tx) in ws_sessions.drain() {
                        let _ = tx.send(LocalWsCommand::Close {
                            code: 1011,
                            reason: "tunnel writer closed".into(),
                        });
                    }
                    let _ = writer_task.await;
                    return Err("tunnel writer closed".into());
                }
            }
            _ = shutdown_rx.changed() => {
                info!("shutting down tunnel");
                for (_, tx) in ws_sessions.drain() {
                    let _ = tx.send(LocalWsCommand::Close {
                        code: 1000,
                        reason: "shutdown".into(),
                    });
                }
                drop(out_tx);
                let _ = writer_task.await;
                return Ok(());
            }
        }
    }
}
