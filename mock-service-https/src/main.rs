use axum::{
    extract::{Json, Path},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use clap::Parser;
use rustls::ServerConfig;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "mock-service-https",
    about = "Self-signed HTTPS echo server for rs-rok integration tests"
)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value_t = 9443)]
    port: u16,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    let args = Args::parse();

    let app = Router::new()
        .route("/echo", get(echo_get).post(echo_post))
        .route("/status/{code}", get(status_handler))
        .route("/slow/{ms}", get(slow_handler))
        .route("/health", get(health));

    let tls_config = generate_self_signed_tls();
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    tracing::info!("mock-service-https listening on https://{}", addr);

    axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service())
        .await
        .expect("server error");
}

/// Generate a self-signed certificate and return a rustls `RustlsConfig` for axum-server.
fn generate_self_signed_tls() -> axum_server::tls_rustls::RustlsConfig {
    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ])
    .expect("failed to generate self-signed certificate");

    let cert_der = cert.cert.der().clone();
    let key_der =
        rustls_pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

    let mut server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert_der.into()],
            rustls_pki_types::PrivateKeyDer::Pkcs8(key_der),
        )
        .expect("failed to build rustls ServerConfig");
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
}

async fn echo_get(headers: HeaderMap) -> impl IntoResponse {
    let header_map: serde_json::Map<String, Value> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                Value::String(v.to_str().unwrap_or("").to_owned()),
            )
        })
        .collect();

    Json(json!({
        "method": "GET",
        "headers": header_map,
        "body": null,
        "tls": true
    }))
}

async fn echo_post(headers: HeaderMap, body: String) -> impl IntoResponse {
    let header_map: serde_json::Map<String, Value> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                Value::String(v.to_str().unwrap_or("").to_owned()),
            )
        })
        .collect();

    Json(json!({
        "method": "POST",
        "headers": header_map,
        "body": body,
        "tls": true
    }))
}

async fn status_handler(Path(code): Path<u16>) -> impl IntoResponse {
    let status = StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_REQUEST);
    (status, format!("Status: {}", code))
}

async fn slow_handler(Path(ms): Path<u64>) -> impl IntoResponse {
    let capped = ms.min(30_000);
    tokio::time::sleep(std::time::Duration::from_millis(capped)).await;
    Json(json!({ "delayed_ms": capped, "tls": true }))
}

async fn health() -> &'static str {
    "ok"
}
