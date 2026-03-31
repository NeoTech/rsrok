use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use rs_rok_protocol::{Frame, Header, Method};
use std::fmt;
use tokio::sync::mpsc;

/// Which scheme to use when forwarding requests to the local service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardScheme {
    Http,
    Https,
}

type HyperResponse = hyper::Response<hyper::body::Incoming>;

#[derive(Debug)]
pub enum ProxyError {
    InvalidFrame(&'static str),
    Http(String),
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyError::InvalidFrame(msg) => write!(f, "invalid frame: {msg}"),
            ProxyError::Http(msg) => write!(f, "HTTP error: {msg}"),
        }
    }
}

impl std::error::Error for ProxyError {}

fn protocol_method_to_http(method: &Method) -> hyper::Method {
    match method {
        Method::Get => hyper::Method::GET,
        Method::Post => hyper::Method::POST,
        Method::Put => hyper::Method::PUT,
        Method::Delete => hyper::Method::DELETE,
        Method::Patch => hyper::Method::PATCH,
        Method::Head => hyper::Method::HEAD,
        Method::Options => hyper::Method::OPTIONS,
        Method::Connect => hyper::Method::CONNECT,
        Method::Trace => hyper::Method::TRACE,
    }
}

/// Returns true if the response should be streamed rather than buffered.
fn is_streaming_response(headers: &hyper::HeaderMap) -> bool {
    if let Some(ct) = headers.get(hyper::header::CONTENT_TYPE) {
        if let Ok(ct_str) = ct.to_str() {
            let lower = ct_str.to_ascii_lowercase();
            if lower.starts_with("text/event-stream") {
                return true;
            }
        }
    }
    false
}

/// Forward a REQUEST frame to the local service and return a RESPONSE frame.
/// For streaming responses (SSE), sends frames directly to `out_tx` and returns None.
pub async fn forward_request(
    frame: &Frame,
    local_addr: &str,
    scheme: ForwardScheme,
    out_tx: &mpsc::UnboundedSender<Frame>,
) -> Result<Option<Frame>, ProxyError> {
    let (request_id, method, url, headers, body) = match frame {
        Frame::Request {
            request_id,
            method,
            url,
            headers,
            body,
        } => (*request_id, method, url, headers, body),
        _ => return Err(ProxyError::InvalidFrame("expected REQUEST frame")),
    };

    let scheme_str = match scheme {
        ForwardScheme::Http => "http",
        ForwardScheme::Https => "https",
    };
    let target_uri = format!("{scheme_str}://{local_addr}{url}");
    let uri: hyper::Uri = target_uri
        .parse()
        .map_err(|e: hyper::http::uri::InvalidUri| ProxyError::Http(e.to_string()))?;

    let mut builder = hyper::Request::builder()
        .method(protocol_method_to_http(method))
        .uri(uri);

    for h in headers {
        // Don't advertise compression support — the tunnel transports raw bytes,
        // so we need the local server to respond with an uncompressed body.
        if h.name.eq_ignore_ascii_case("accept-encoding") {
            continue;
        }
        let name = hyper::header::HeaderName::from_bytes(h.name.as_bytes())
            .map_err(|e| ProxyError::Http(e.to_string()))?;
        let value = hyper::header::HeaderValue::from_str(&h.value)
            .map_err(|e| ProxyError::Http(e.to_string()))?;
        builder = builder.header(name, value);
    }

    let req = builder
        .body(Full::new(Bytes::from(body.clone())))
        .map_err(|e| ProxyError::Http(e.to_string()))?;

    let resp = send_request(req, scheme).await?;

    let status = resp.status().as_u16();

    // Strip hop-by-hop headers — these describe the local TCP connection and
    // must not be forwarded to the browser over the tunnel.
    const HOP_BY_HOP: &[&str] = &[
        "connection",
        "keep-alive",
        "transfer-encoding",
        "upgrade",
        "proxy-connection",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        // Remove content-encoding: the tunnel carries raw uncompressed bytes.
        // If the local server somehow still replies compressed, the browser
        // would try to decompress an already-decoded or re-encoded body.
        "content-encoding",
    ];

    let resp_headers: Vec<Header> = resp
        .headers()
        .iter()
        .filter(|(k, _)| !HOP_BY_HOP.contains(&k.as_str()))
        .map(|(k, v)| Header {
            name: k.as_str().to_string(),
            value: v.to_str().unwrap_or("").to_string(),
        })
        .collect();

    // Streaming (SSE) responses: send headers immediately, then stream chunks
    if is_streaming_response(resp.headers()) {
        let _ = out_tx.send(Frame::StreamStart {
            request_id,
            status,
            headers: resp_headers,
        });

        let mut body_stream = resp.into_body();
        loop {
            match body_stream.frame().await {
                Some(Ok(frame)) => {
                    if let Some(data) = frame.data_ref() {
                        if out_tx
                            .send(Frame::StreamData {
                                request_id,
                                data: data.to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                Some(Err(_)) | None => break,
            }
        }

        let _ = out_tx.send(Frame::StreamEnd { request_id });
        return Ok(None);
    }

    // Buffered response: collect entire body
    let resp_body = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| ProxyError::Http(e.to_string()))?
        .to_bytes()
        .to_vec();

    Ok(Some(Frame::Response {
        request_id,
        status,
        headers: resp_headers,
        body: resp_body,
    }))
}

/// Collect the full error cause chain into a single string.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut msg = err.to_string();
    let mut source = err.source();
    while let Some(s) = source {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        source = s.source();
    }
    msg
}

/// Send an HTTP(S) request to the local service. Uses a TLS connector that
/// accepts self-signed certificates when scheme is HTTPS (local dev servers).
async fn send_request(
    req: hyper::Request<Full<Bytes>>,
    scheme: ForwardScheme,
) -> Result<HyperResponse, ProxyError> {
    match scheme {
        ForwardScheme::Http => {
            let client = Client::builder(TokioExecutor::new())
                .build(hyper_util::client::legacy::connect::HttpConnector::new());
            client
                .request(req)
                .await
                .map_err(|e| ProxyError::Http(error_chain(&e)))
        }
        ForwardScheme::Https => {
            let tls = native_tls::TlsConnector::builder()
                .danger_accept_invalid_certs(true)
                .build()
                .map_err(|e| ProxyError::Http(error_chain(&e)))?;
            let mut http = hyper_util::client::legacy::connect::HttpConnector::new();
            http.enforce_http(false);
            let connector = hyper_tls::HttpsConnector::from((http, tls.into()));
            let client = Client::builder(TokioExecutor::new()).build(connector);
            client
                .request(req)
                .await
                .map_err(|e| ProxyError::Http(error_chain(&e)))
        }
    }
}
