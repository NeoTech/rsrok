use wasm_bindgen::prelude::*;

use rs_rok_protocol::{self as proto, Header};

/// Parse a binary frame from a `Uint8Array`. Returns a JS object
/// representation of the frame, or throws on error.
#[wasm_bindgen]
pub fn parse_frame(data: &[u8]) -> Result<JsValue, JsValue> {
    let (frame, consumed) = proto::decode(data).map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
    let obj = js_sys::Object::new();

    js_sys::Reflect::set(&obj, &"consumed".into(), &JsValue::from_f64(consumed as f64))?;
    js_sys::Reflect::set(
        &obj,
        &"frameType".into(),
        &JsValue::from_f64(frame.frame_type() as f64),
    )?;
    js_sys::Reflect::set(
        &obj,
        &"requestId".into(),
        &JsValue::from_f64(frame.request_id() as f64),
    )?;

    match &frame {
        proto::Frame::Register {
            tunnel_id,
            auth_token,
            tunnel_type,
            ..
        } => {
            js_sys::Reflect::set(
                &obj,
                &"tunnelId".into(),
                &js_sys::Uint8Array::from(tunnel_id.as_slice()).into(),
            )?;
            js_sys::Reflect::set(
                &obj,
                &"authToken".into(),
                &js_sys::Uint8Array::from(auth_token.as_slice()).into(),
            )?;
            js_sys::Reflect::set(
                &obj,
                &"tunnelType".into(),
                &JsValue::from_f64(*tunnel_type as u8 as f64),
            )?;
        }
        proto::Frame::RegisterAck {
            tunnel_id,
            public_url,
            ..
        } => {
            js_sys::Reflect::set(
                &obj,
                &"tunnelId".into(),
                &js_sys::Uint8Array::from(tunnel_id.as_slice()).into(),
            )?;
            js_sys::Reflect::set(&obj, &"publicUrl".into(), &JsValue::from_str(public_url))?;
        }
        proto::Frame::Request {
            method,
            url,
            headers,
            body,
            ..
        } => {
            js_sys::Reflect::set(
                &obj,
                &"method".into(),
                &JsValue::from_f64(*method as u8 as f64),
            )?;
            js_sys::Reflect::set(&obj, &"url".into(), &JsValue::from_str(url))?;
            js_sys::Reflect::set(&obj, &"headers".into(), &headers_to_js(headers)?)?;
            js_sys::Reflect::set(
                &obj,
                &"body".into(),
                &js_sys::Uint8Array::from(body.as_slice()).into(),
            )?;
        }
        proto::Frame::Response {
            status,
            headers,
            body,
            ..
        } => {
            js_sys::Reflect::set(
                &obj,
                &"status".into(),
                &JsValue::from_f64(*status as f64),
            )?;
            js_sys::Reflect::set(&obj, &"headers".into(), &headers_to_js(headers)?)?;
            js_sys::Reflect::set(
                &obj,
                &"body".into(),
                &js_sys::Uint8Array::from(body.as_slice()).into(),
            )?;
        }
        proto::Frame::Ping { .. } | proto::Frame::Pong { .. } => {}
        proto::Frame::Error { code, message, .. } => {
            js_sys::Reflect::set(&obj, &"code".into(), &JsValue::from_f64(*code as f64))?;
            js_sys::Reflect::set(&obj, &"message".into(), &JsValue::from_str(message))?;
        }
        proto::Frame::WsOpen {
            ws_id,
            url,
            headers,
            protocols,
            ..
        } => {
            js_sys::Reflect::set(&obj, &"wsId".into(), &JsValue::from_f64(*ws_id as f64))?;
            js_sys::Reflect::set(&obj, &"url".into(), &JsValue::from_str(url))?;
            js_sys::Reflect::set(&obj, &"headers".into(), &headers_to_js(headers)?)?;
            js_sys::Reflect::set(&obj, &"protocols".into(), &strings_to_js(protocols)?)?;
        }
        proto::Frame::WsData {
            ws_id,
            is_binary,
            data,
            ..
        } => {
            js_sys::Reflect::set(&obj, &"wsId".into(), &JsValue::from_f64(*ws_id as f64))?;
            js_sys::Reflect::set(&obj, &"isBinary".into(), &JsValue::from_bool(*is_binary))?;
            js_sys::Reflect::set(
                &obj,
                &"data".into(),
                &js_sys::Uint8Array::from(data.as_slice()).into(),
            )?;
        }
        proto::Frame::WsClose {
            ws_id,
            code,
            reason,
            ..
        } => {
            js_sys::Reflect::set(&obj, &"wsId".into(), &JsValue::from_f64(*ws_id as f64))?;
            js_sys::Reflect::set(&obj, &"code".into(), &JsValue::from_f64(*code as f64))?;
            js_sys::Reflect::set(&obj, &"reason".into(), &JsValue::from_str(reason))?;
        }
        proto::Frame::StreamStart {
            status, headers, ..
        } => {
            js_sys::Reflect::set(
                &obj,
                &"status".into(),
                &JsValue::from_f64(*status as f64),
            )?;
            js_sys::Reflect::set(&obj, &"headers".into(), &headers_to_js(headers)?)?;
        }
        proto::Frame::StreamData { data, .. } => {
            js_sys::Reflect::set(
                &obj,
                &"data".into(),
                &js_sys::Uint8Array::from(data.as_slice()).into(),
            )?;
        }
        proto::Frame::StreamEnd { .. } => {}
        proto::Frame::TcpOpen {
            stream_id, token, ..
        } => {
            js_sys::Reflect::set(
                &obj,
                &"streamId".into(),
                &JsValue::from_f64(*stream_id as f64),
            )?;
            js_sys::Reflect::set(&obj, &"token".into(), &JsValue::from_str(token))?;
        }
        proto::Frame::TcpOpenAck { stream_id, .. } => {
            js_sys::Reflect::set(
                &obj,
                &"streamId".into(),
                &JsValue::from_f64(*stream_id as f64),
            )?;
        }
        proto::Frame::TcpData {
            stream_id, data, ..
        } => {
            js_sys::Reflect::set(
                &obj,
                &"streamId".into(),
                &JsValue::from_f64(*stream_id as f64),
            )?;
            js_sys::Reflect::set(
                &obj,
                &"data".into(),
                &js_sys::Uint8Array::from(data.as_slice()).into(),
            )?;
        }
        proto::Frame::TcpClose {
            stream_id, reason, ..
        } => {
            js_sys::Reflect::set(
                &obj,
                &"streamId".into(),
                &JsValue::from_f64(*stream_id as f64),
            )?;
            js_sys::Reflect::set(&obj, &"reason".into(), &JsValue::from_str(reason))?;
        }
    }

    Ok(obj.into())
}

/// Encode a RESPONSE frame from individual fields into a `Uint8Array`.
#[wasm_bindgen]
pub fn encode_response(
    request_id: u32,
    status: u16,
    header_names: Vec<String>,
    header_values: Vec<String>,
    body: &[u8],
) -> Result<js_sys::Uint8Array, JsValue> {
    let headers: Vec<Header> = header_names
        .into_iter()
        .zip(header_values)
        .map(|(name, value)| Header { name, value })
        .collect();

    let frame = proto::Frame::Response {
        request_id,
        status,
        headers,
        body: body.to_vec(),
    };
    let bytes = proto::encode(&frame);
    Ok(js_sys::Uint8Array::from(bytes.as_slice()))
}

/// Encode a REQUEST frame from individual fields into a `Uint8Array`.
#[wasm_bindgen]
pub fn encode_request(
    request_id: u32,
    method: u8,
    url: &str,
    header_names: Vec<String>,
    header_values: Vec<String>,
    body: &[u8],
) -> Result<js_sys::Uint8Array, JsValue> {
    let method =
        proto::Method::from_u8(method).map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
    let headers: Vec<Header> = header_names
        .into_iter()
        .zip(header_values)
        .map(|(name, value)| Header { name, value })
        .collect();

    let frame = proto::Frame::Request {
        request_id,
        method,
        url: url.into(),
        headers,
        body: body.to_vec(),
    };
    let bytes = proto::encode(&frame);
    Ok(js_sys::Uint8Array::from(bytes.as_slice()))
}

/// Encode a REGISTER_ACK frame.
#[wasm_bindgen]
pub fn encode_register_ack(
    request_id: u32,
    tunnel_id: &[u8],
    public_url: &str,
) -> Result<js_sys::Uint8Array, JsValue> {
    if tunnel_id.len() != 16 {
        return Err(JsValue::from_str("tunnel_id must be 16 bytes"));
    }
    let mut tid = [0u8; 16];
    tid.copy_from_slice(tunnel_id);

    let frame = proto::Frame::RegisterAck {
        request_id,
        tunnel_id: tid,
        public_url: public_url.into(),
    };
    let bytes = proto::encode(&frame);
    Ok(js_sys::Uint8Array::from(bytes.as_slice()))
}

/// Encode a PING frame.
#[wasm_bindgen]
pub fn encode_ping(request_id: u32) -> js_sys::Uint8Array {
    let bytes = proto::encode(&proto::Frame::Ping { request_id });
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a PONG frame.
#[wasm_bindgen]
pub fn encode_pong(request_id: u32) -> js_sys::Uint8Array {
    let bytes = proto::encode(&proto::Frame::Pong { request_id });
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode an ERROR frame.
#[wasm_bindgen]
pub fn encode_error(request_id: u32, code: u16, message: &str) -> js_sys::Uint8Array {
    let frame = proto::Frame::Error {
        request_id,
        code,
        message: message.into(),
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a WS_OPEN frame.
#[wasm_bindgen]
pub fn encode_ws_open(
    request_id: u32,
    ws_id: u32,
    url: &str,
    header_names: Vec<String>,
    header_values: Vec<String>,
    protocols: Vec<String>,
) -> js_sys::Uint8Array {
    let headers: Vec<Header> = header_names
        .into_iter()
        .zip(header_values)
        .map(|(name, value)| Header { name, value })
        .collect();

    let frame = proto::Frame::WsOpen {
        request_id,
        ws_id,
        url: url.into(),
        headers,
        protocols,
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a WS_DATA frame.
#[wasm_bindgen]
pub fn encode_ws_data(
    request_id: u32,
    ws_id: u32,
    is_binary: bool,
    data: &[u8],
) -> js_sys::Uint8Array {
    let frame = proto::Frame::WsData {
        request_id,
        ws_id,
        is_binary,
        data: data.to_vec(),
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a WS_CLOSE frame.
#[wasm_bindgen]
pub fn encode_ws_close(
    request_id: u32,
    ws_id: u32,
    code: u16,
    reason: &str,
) -> js_sys::Uint8Array {
    let frame = proto::Frame::WsClose {
        request_id,
        ws_id,
        code,
        reason: reason.into(),
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a STREAM_START frame.
#[wasm_bindgen]
pub fn encode_stream_start(
    request_id: u32,
    status: u16,
    header_names: Vec<String>,
    header_values: Vec<String>,
) -> js_sys::Uint8Array {
    let headers: Vec<Header> = header_names
        .into_iter()
        .zip(header_values)
        .map(|(name, value)| Header { name, value })
        .collect();
    let frame = proto::Frame::StreamStart {
        request_id,
        status,
        headers,
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a STREAM_DATA frame.
#[wasm_bindgen]
pub fn encode_stream_data(request_id: u32, data: &[u8]) -> js_sys::Uint8Array {
    let frame = proto::Frame::StreamData {
        request_id,
        data: data.to_vec(),
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a STREAM_END frame.
#[wasm_bindgen]
pub fn encode_stream_end(request_id: u32) -> js_sys::Uint8Array {
    let bytes = proto::encode(&proto::Frame::StreamEnd { request_id });
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a TCP_OPEN frame.
#[wasm_bindgen]
pub fn encode_tcp_open(request_id: u32, stream_id: u32, token: &str) -> js_sys::Uint8Array {
    let frame = proto::Frame::TcpOpen {
        request_id,
        stream_id,
        token: token.into(),
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a TCP_OPEN_ACK frame.
#[wasm_bindgen]
pub fn encode_tcp_open_ack(request_id: u32, stream_id: u32) -> js_sys::Uint8Array {
    let frame = proto::Frame::TcpOpenAck {
        request_id,
        stream_id,
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a TCP_DATA frame.
#[wasm_bindgen]
pub fn encode_tcp_data(request_id: u32, stream_id: u32, data: &[u8]) -> js_sys::Uint8Array {
    let frame = proto::Frame::TcpData {
        request_id,
        stream_id,
        data: data.to_vec(),
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

/// Encode a TCP_CLOSE frame.
#[wasm_bindgen]
pub fn encode_tcp_close(request_id: u32, stream_id: u32, reason: &str) -> js_sys::Uint8Array {
    let frame = proto::Frame::TcpClose {
        request_id,
        stream_id,
        reason: reason.into(),
    };
    let bytes = proto::encode(&frame);
    js_sys::Uint8Array::from(bytes.as_slice())
}

fn headers_to_js(headers: &[Header]) -> Result<JsValue, JsValue> {
    let arr = js_sys::Array::new();
    for h in headers {
        let pair = js_sys::Array::new();
        pair.push(&JsValue::from_str(&h.name));
        pair.push(&JsValue::from_str(&h.value));
        arr.push(&pair);
    }
    Ok(arr.into())
}

fn strings_to_js(values: &[String]) -> Result<JsValue, JsValue> {
    let arr = js_sys::Array::new();
    for v in values {
        arr.push(&JsValue::from_str(v));
    }
    Ok(arr.into())
}
