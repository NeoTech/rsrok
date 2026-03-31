#![cfg_attr(not(feature = "std"), no_std)]

//! rs-rok binary framing protocol.
//!
//! Frame layout: `[1B type][4B request_id LE][4B payload_len LE][payload]`

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

/// Header size: 1 byte type + 4 bytes request_id + 4 bytes payload_len
pub const HEADER_SIZE: usize = 9;

// Frame type constants
pub const FRAME_REGISTER: u8 = 0x01;
pub const FRAME_REGISTER_ACK: u8 = 0x02;
pub const FRAME_REQUEST: u8 = 0x03;
pub const FRAME_RESPONSE: u8 = 0x04;
pub const FRAME_PING: u8 = 0x05;
pub const FRAME_PONG: u8 = 0x06;
pub const FRAME_ERROR: u8 = 0x07;
pub const FRAME_WS_OPEN: u8 = 0x08;
pub const FRAME_WS_DATA: u8 = 0x09;
pub const FRAME_WS_CLOSE: u8 = 0x0A;
pub const FRAME_STREAM_START: u8 = 0x0B;
pub const FRAME_STREAM_DATA: u8 = 0x0C;
pub const FRAME_STREAM_END: u8 = 0x0D;
pub const FRAME_TCP_OPEN: u8 = 0x0E;
pub const FRAME_TCP_OPEN_ACK: u8 = 0x0F;
pub const FRAME_TCP_DATA: u8 = 0x10;
pub const FRAME_TCP_CLOSE: u8 = 0x11;

/// HTTP method encoded as a single byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Method {
    Get = 0,
    Post = 1,
    Put = 2,
    Delete = 3,
    Patch = 4,
    Head = 5,
    Options = 6,
    Connect = 7,
    Trace = 8,
}

impl Method {
    pub fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(Self::Get),
            1 => Ok(Self::Post),
            2 => Ok(Self::Put),
            3 => Ok(Self::Delete),
            4 => Ok(Self::Patch),
            5 => Ok(Self::Head),
            6 => Ok(Self::Options),
            7 => Ok(Self::Connect),
            8 => Ok(Self::Trace),
            _ => Err(DecodeError::InvalidMethod(v)),
        }
    }
}

/// Tunnel type: HTTP, HTTPS (TLS terminated at edge), or TCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TunnelType {
    Http = 0,
    Https = 1,
    Tcp = 2,
}

impl TunnelType {
    pub fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(Self::Http),
            1 => Ok(Self::Https),
            2 => Ok(Self::Tcp),
            _ => Err(DecodeError::InvalidTunnelType(v)),
        }
    }
}

/// A key-value header pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// All frame types in the rs-rok protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Register {
        request_id: u32,
        tunnel_id: [u8; 16],
        auth_token: [u8; 32],
        tunnel_type: TunnelType,
    },
    RegisterAck {
        request_id: u32,
        tunnel_id: [u8; 16],
        public_url: String,
    },
    Request {
        request_id: u32,
        method: Method,
        url: String,
        headers: Vec<Header>,
        body: Vec<u8>,
    },
    Response {
        request_id: u32,
        status: u16,
        headers: Vec<Header>,
        body: Vec<u8>,
    },
    Ping {
        request_id: u32,
    },
    Pong {
        request_id: u32,
    },
    Error {
        request_id: u32,
        code: u16,
        message: String,
    },
    /// Open a local websocket connection and bind it to `ws_id`.
    WsOpen {
        request_id: u32,
        ws_id: u32,
        url: String,
        headers: Vec<Header>,
        protocols: Vec<String>,
    },
    /// Relay one websocket data message on `ws_id`.
    WsData {
        request_id: u32,
        ws_id: u32,
        is_binary: bool,
        data: Vec<u8>,
    },
    /// Close websocket `ws_id`.
    WsClose {
        request_id: u32,
        ws_id: u32,
        code: u16,
        reason: String,
    },
    /// Begin a streaming HTTP response (status + headers, no body yet).
    StreamStart {
        request_id: u32,
        status: u16,
        headers: Vec<Header>,
    },
    /// A chunk of data for a streaming HTTP response.
    StreamData {
        request_id: u32,
        data: Vec<u8>,
    },
    /// End of a streaming HTTP response.
    StreamEnd {
        request_id: u32,
    },
    /// Open a TCP tunnel stream (client -> server via DO).
    TcpOpen {
        request_id: u32,
        stream_id: u32,
        token: String,
    },
    /// Acknowledge a TCP stream open (server -> client via DO).
    TcpOpenAck {
        request_id: u32,
        stream_id: u32,
    },
    /// Raw TCP data (bidirectional).
    TcpData {
        request_id: u32,
        stream_id: u32,
        data: Vec<u8>,
    },
    /// Close a TCP stream (either direction).
    TcpClose {
        request_id: u32,
        stream_id: u32,
        reason: String,
    },
}

impl Frame {
    pub fn request_id(&self) -> u32 {
        match self {
            Frame::Register { request_id, .. }
            | Frame::RegisterAck { request_id, .. }
            | Frame::Request { request_id, .. }
            | Frame::Response { request_id, .. }
            | Frame::Ping { request_id }
            | Frame::Pong { request_id }
            | Frame::Error { request_id, .. }
            | Frame::WsOpen { request_id, .. }
            | Frame::WsData { request_id, .. }
            | Frame::WsClose { request_id, .. }
            | Frame::StreamStart { request_id, .. }
            | Frame::StreamData { request_id, .. }
            | Frame::StreamEnd { request_id }
            | Frame::TcpOpen { request_id, .. }
            | Frame::TcpOpenAck { request_id, .. }
            | Frame::TcpData { request_id, .. }
            | Frame::TcpClose { request_id, .. } => *request_id,
        }
    }

    pub fn frame_type(&self) -> u8 {
        match self {
            Frame::Register { .. } => FRAME_REGISTER,
            Frame::RegisterAck { .. } => FRAME_REGISTER_ACK,
            Frame::Request { .. } => FRAME_REQUEST,
            Frame::Response { .. } => FRAME_RESPONSE,
            Frame::Ping { .. } => FRAME_PING,
            Frame::Pong { .. } => FRAME_PONG,
            Frame::Error { .. } => FRAME_ERROR,
            Frame::WsOpen { .. } => FRAME_WS_OPEN,
            Frame::WsData { .. } => FRAME_WS_DATA,
            Frame::WsClose { .. } => FRAME_WS_CLOSE,
            Frame::StreamStart { .. } => FRAME_STREAM_START,
            Frame::StreamData { .. } => FRAME_STREAM_DATA,
            Frame::StreamEnd { .. } => FRAME_STREAM_END,
            Frame::TcpOpen { .. } => FRAME_TCP_OPEN,
            Frame::TcpOpenAck { .. } => FRAME_TCP_OPEN_ACK,
            Frame::TcpData { .. } => FRAME_TCP_DATA,
            Frame::TcpClose { .. } => FRAME_TCP_CLOSE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Not enough bytes to parse; need more data.
    Incomplete,
    /// Unknown frame type byte.
    UnknownFrameType(u8),
    /// Invalid HTTP method byte.
    InvalidMethod(u8),
    /// Invalid tunnel type byte.
    InvalidTunnelType(u8),
    /// Invalid UTF-8 in a string field.
    InvalidUtf8,
    /// Payload too short for the declared frame contents.
    PayloadTooShort,
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

fn write_u16_le(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u16_le(buf, s.len() as u16);
    buf.extend_from_slice(s.as_bytes());
}

fn write_headers(buf: &mut Vec<u8>, headers: &[Header]) {
    write_u16_le(buf, headers.len() as u16);
    for h in headers {
        write_str(buf, &h.name);
        write_str(buf, &h.value);
    }
}

fn write_strings(buf: &mut Vec<u8>, values: &[String]) {
    write_u16_le(buf, values.len() as u16);
    for v in values {
        write_str(buf, v);
    }
}

/// Encode a frame into bytes.
pub fn encode(frame: &Frame) -> Vec<u8> {
    let mut payload = Vec::new();

    match frame {
        Frame::Register {
            tunnel_id,
            auth_token,
            tunnel_type,
            ..
        } => {
            payload.extend_from_slice(tunnel_id);
            payload.extend_from_slice(auth_token);
            payload.push(*tunnel_type as u8);
        }
        Frame::RegisterAck {
            tunnel_id,
            public_url,
            ..
        } => {
            payload.extend_from_slice(tunnel_id);
            write_str(&mut payload, public_url);
        }
        Frame::Request {
            method,
            url,
            headers,
            body,
            ..
        } => {
            payload.push(*method as u8);
            write_str(&mut payload, url);
            write_headers(&mut payload, headers);
            write_u32_le(&mut payload, body.len() as u32);
            payload.extend_from_slice(body);
        }
        Frame::Response {
            status,
            headers,
            body,
            ..
        } => {
            write_u16_le(&mut payload, *status);
            write_headers(&mut payload, headers);
            write_u32_le(&mut payload, body.len() as u32);
            payload.extend_from_slice(body);
        }
        Frame::Ping { .. } | Frame::Pong { .. } => {
            // empty payload
        }
        Frame::Error { code, message, .. } => {
            write_u16_le(&mut payload, *code);
            write_str(&mut payload, message);
        }
        Frame::WsOpen {
            ws_id,
            url,
            headers,
            protocols,
            ..
        } => {
            write_u32_le(&mut payload, *ws_id);
            write_str(&mut payload, url);
            write_headers(&mut payload, headers);
            write_strings(&mut payload, protocols);
        }
        Frame::WsData {
            ws_id,
            is_binary,
            data,
            ..
        } => {
            write_u32_le(&mut payload, *ws_id);
            payload.push(u8::from(*is_binary));
            write_u32_le(&mut payload, data.len() as u32);
            payload.extend_from_slice(data);
        }
        Frame::WsClose {
            ws_id,
            code,
            reason,
            ..
        } => {
            write_u32_le(&mut payload, *ws_id);
            write_u16_le(&mut payload, *code);
            write_str(&mut payload, reason);
        }
        Frame::StreamStart {
            status, headers, ..
        } => {
            write_u16_le(&mut payload, *status);
            write_headers(&mut payload, headers);
        }
        Frame::StreamData { data, .. } => {
            write_u32_le(&mut payload, data.len() as u32);
            payload.extend_from_slice(data);
        }
        Frame::StreamEnd { .. } => {
            // empty payload
        }
        Frame::TcpOpen {
            stream_id, token, ..
        } => {
            write_u32_le(&mut payload, *stream_id);
            write_str(&mut payload, token);
        }
        Frame::TcpOpenAck { stream_id, .. } => {
            write_u32_le(&mut payload, *stream_id);
        }
        Frame::TcpData {
            stream_id, data, ..
        } => {
            write_u32_le(&mut payload, *stream_id);
            write_u32_le(&mut payload, data.len() as u32);
            payload.extend_from_slice(data);
        }
        Frame::TcpClose {
            stream_id, reason, ..
        } => {
            write_u32_le(&mut payload, *stream_id);
            write_str(&mut payload, reason);
        }
    }

    let mut buf = Vec::with_capacity(HEADER_SIZE + payload.len());
    buf.push(frame.frame_type());
    write_u32_le(&mut buf, frame.request_id());
    write_u32_le(&mut buf, payload.len() as u32);
    buf.extend_from_slice(&payload);
    buf
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn read_u8(&mut self) -> Result<u8, DecodeError> {
        if self.remaining() < 1 {
            return Err(DecodeError::PayloadTooShort);
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn read_u16_le(&mut self) -> Result<u16, DecodeError> {
        if self.remaining() < 2 {
            return Err(DecodeError::PayloadTooShort);
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn read_u32_le(&mut self) -> Result<u32, DecodeError> {
        if self.remaining() < 4 {
            return Err(DecodeError::PayloadTooShort);
        }
        let v = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            return Err(DecodeError::PayloadTooShort);
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn read_str(&mut self) -> Result<String, DecodeError> {
        let len = self.read_u16_le()? as usize;
        let bytes = self.read_bytes(len)?;
        core::str::from_utf8(bytes)
            .map(|s| s.into())
            .map_err(|_| DecodeError::InvalidUtf8)
    }

    fn read_headers(&mut self) -> Result<Vec<Header>, DecodeError> {
        let count = self.read_u16_le()? as usize;
        let mut headers = Vec::with_capacity(count);
        for _ in 0..count {
            let name = self.read_str()?;
            let value = self.read_str()?;
            headers.push(Header { name, value });
        }
        Ok(headers)
    }

    fn read_strings(&mut self) -> Result<Vec<String>, DecodeError> {
        let count = self.read_u16_le()? as usize;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(self.read_str()?);
        }
        Ok(values)
    }

    fn read_fixed<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let bytes = self.read_bytes(N)?;
        let mut arr = [0u8; N];
        arr.copy_from_slice(bytes);
        Ok(arr)
    }
}

/// Decode a single frame from the front of `data`.
///
/// Returns the decoded frame and the total number of bytes consumed
/// (header + payload). If you have a stream of frames, advance your
/// buffer by that many bytes.
///
/// Returns `Err(DecodeError::Incomplete)` if there are not enough bytes
/// to parse even the header or the full payload.
pub fn decode(data: &[u8]) -> Result<(Frame, usize), DecodeError> {
    if data.len() < HEADER_SIZE {
        return Err(DecodeError::Incomplete);
    }

    let frame_type = data[0];
    let request_id = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
    let payload_len = u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as usize;

    let total_len = HEADER_SIZE + payload_len;
    if data.len() < total_len {
        return Err(DecodeError::Incomplete);
    }

    let payload = &data[HEADER_SIZE..total_len];
    let mut r = Reader::new(payload);

    let frame = match frame_type {
        FRAME_REGISTER => {
            let tunnel_id = r.read_fixed::<16>()?;
            let auth_token = r.read_fixed::<32>()?;
            let tt = r.read_u8()?;
            Frame::Register {
                request_id,
                tunnel_id,
                auth_token,
                tunnel_type: TunnelType::from_u8(tt)?,
            }
        }
        FRAME_REGISTER_ACK => {
            let tunnel_id = r.read_fixed::<16>()?;
            let public_url = r.read_str()?;
            Frame::RegisterAck {
                request_id,
                tunnel_id,
                public_url,
            }
        }
        FRAME_REQUEST => {
            let method_byte = r.read_u8()?;
            let method = Method::from_u8(method_byte)?;
            let url = r.read_str()?;
            let headers = r.read_headers()?;
            let body_len = r.read_u32_le()? as usize;
            let body = r.read_bytes(body_len)?.to_vec();
            Frame::Request {
                request_id,
                method,
                url,
                headers,
                body,
            }
        }
        FRAME_RESPONSE => {
            let status = r.read_u16_le()?;
            let headers = r.read_headers()?;
            let body_len = r.read_u32_le()? as usize;
            let body = r.read_bytes(body_len)?.to_vec();
            Frame::Response {
                request_id,
                status,
                headers,
                body,
            }
        }
        FRAME_PING => Frame::Ping { request_id },
        FRAME_PONG => Frame::Pong { request_id },
        FRAME_ERROR => {
            let code = r.read_u16_le()?;
            let message = r.read_str()?;
            Frame::Error {
                request_id,
                code,
                message,
            }
        }
        FRAME_WS_OPEN => {
            let ws_id = r.read_u32_le()?;
            let url = r.read_str()?;
            let headers = r.read_headers()?;
            let protocols = r.read_strings()?;
            Frame::WsOpen {
                request_id,
                ws_id,
                url,
                headers,
                protocols,
            }
        }
        FRAME_WS_DATA => {
            let ws_id = r.read_u32_le()?;
            let is_binary = r.read_u8()? != 0;
            let data_len = r.read_u32_le()? as usize;
            let data = r.read_bytes(data_len)?.to_vec();
            Frame::WsData {
                request_id,
                ws_id,
                is_binary,
                data,
            }
        }
        FRAME_WS_CLOSE => {
            let ws_id = r.read_u32_le()?;
            let code = r.read_u16_le()?;
            let reason = r.read_str()?;
            Frame::WsClose {
                request_id,
                ws_id,
                code,
                reason,
            }
        }
        FRAME_STREAM_START => {
            let status = r.read_u16_le()?;
            let headers = r.read_headers()?;
            Frame::StreamStart {
                request_id,
                status,
                headers,
            }
        }
        FRAME_STREAM_DATA => {
            let data_len = r.read_u32_le()? as usize;
            let data = r.read_bytes(data_len)?.to_vec();
            Frame::StreamData {
                request_id,
                data,
            }
        }
        FRAME_STREAM_END => Frame::StreamEnd { request_id },
        FRAME_TCP_OPEN => {
            let stream_id = r.read_u32_le()?;
            let token = r.read_str()?;
            Frame::TcpOpen {
                request_id,
                stream_id,
                token,
            }
        }
        FRAME_TCP_OPEN_ACK => {
            let stream_id = r.read_u32_le()?;
            Frame::TcpOpenAck {
                request_id,
                stream_id,
            }
        }
        FRAME_TCP_DATA => {
            let stream_id = r.read_u32_le()?;
            let data_len = r.read_u32_le()? as usize;
            let data = r.read_bytes(data_len)?.to_vec();
            Frame::TcpData {
                request_id,
                stream_id,
                data,
            }
        }
        FRAME_TCP_CLOSE => {
            let stream_id = r.read_u32_le()?;
            let reason = r.read_str()?;
            Frame::TcpClose {
                request_id,
                stream_id,
                reason,
            }
        }
        _ => return Err(DecodeError::UnknownFrameType(frame_type)),
    };

    Ok((frame, total_len))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(frame: &Frame) {
        let bytes = encode(frame);
        let (decoded, consumed) = decode(&bytes).expect("decode should succeed");
        assert_eq!(consumed, bytes.len(), "should consume all bytes");
        assert_eq!(&decoded, frame, "round-trip mismatch");
    }

    #[test]
    fn register_round_trip() {
        let frame = Frame::Register {
            request_id: 1,
            tunnel_id: [0xAA; 16],
            auth_token: [0xBB; 32],
            tunnel_type: TunnelType::Http,
        };
        round_trip(&frame);
    }

    #[test]
    fn register_https_round_trip() {
        let frame = Frame::Register {
            request_id: 42,
            tunnel_id: [0x01; 16],
            auth_token: [0x02; 32],
            tunnel_type: TunnelType::Https,
        };
        round_trip(&frame);
    }

    #[test]
    fn register_ack_round_trip() {
        let frame = Frame::RegisterAck {
            request_id: 2,
            tunnel_id: [0xCC; 16],
            public_url: "https://abc123.workers.dev".into(),
        };
        round_trip(&frame);
    }

    #[test]
    fn request_round_trip() {
        let frame = Frame::Request {
            request_id: 100,
            method: Method::Post,
            url: "/api/data?foo=bar".into(),
            headers: vec![
                Header {
                    name: "Content-Type".into(),
                    value: "application/json".into(),
                },
                Header {
                    name: "Authorization".into(),
                    value: "Bearer tok123".into(),
                },
            ],
            body: b"{\"key\":\"value\"}".to_vec(),
        };
        round_trip(&frame);
    }

    #[test]
    fn request_empty_body_round_trip() {
        let frame = Frame::Request {
            request_id: 101,
            method: Method::Get,
            url: "/health".into(),
            headers: vec![],
            body: vec![],
        };
        round_trip(&frame);
    }

    #[test]
    fn response_round_trip() {
        let frame = Frame::Response {
            request_id: 100,
            status: 200,
            headers: vec![Header {
                name: "Content-Type".into(),
                value: "text/plain".into(),
            }],
            body: b"Hello, world!".to_vec(),
        };
        round_trip(&frame);
    }

    #[test]
    fn response_empty_body_round_trip() {
        let frame = Frame::Response {
            request_id: 200,
            status: 204,
            headers: vec![],
            body: vec![],
        };
        round_trip(&frame);
    }

    #[test]
    fn ping_round_trip() {
        round_trip(&Frame::Ping { request_id: 0 });
    }

    #[test]
    fn pong_round_trip() {
        round_trip(&Frame::Pong { request_id: 0 });
    }

    #[test]
    fn error_round_trip() {
        let frame = Frame::Error {
            request_id: 5,
            code: 401,
            message: "unauthorized".into(),
        };
        round_trip(&frame);
    }

    #[test]
    fn error_empty_message_round_trip() {
        let frame = Frame::Error {
            request_id: 6,
            code: 500,
            message: String::new(),
        };
        round_trip(&frame);
    }

    #[test]
    fn ws_open_round_trip() {
        let frame = Frame::WsOpen {
            request_id: 77,
            ws_id: 9001,
            url: "/socket.io/?EIO=4&transport=websocket".into(),
            headers: vec![Header {
                name: "cookie".into(),
                value: "sid=abc".into(),
            }],
            protocols: vec!["graphql-transport-ws".into(), "chat".into()],
        };
        round_trip(&frame);
    }

    #[test]
    fn ws_data_round_trip() {
        let frame = Frame::WsData {
            request_id: 78,
            ws_id: 9001,
            is_binary: true,
            data: vec![1, 2, 3, 4, 5],
        };
        round_trip(&frame);
    }

    #[test]
    fn ws_close_round_trip() {
        let frame = Frame::WsClose {
            request_id: 79,
            ws_id: 9001,
            code: 1000,
            reason: "normal closure".into(),
        };
        round_trip(&frame);
    }

    #[test]
    fn incomplete_header() {
        let result = decode(&[0x01, 0x00, 0x00]);
        assert_eq!(result, Err(DecodeError::Incomplete));
    }

    #[test]
    fn incomplete_payload() {
        // Valid header claiming 100 bytes of payload, but only 1 byte present
        let mut buf = vec![FRAME_PING, 0, 0, 0, 0];
        buf.extend_from_slice(&100u32.to_le_bytes());
        buf.push(0xFF);
        assert_eq!(decode(&buf), Err(DecodeError::Incomplete));
    }

    #[test]
    fn unknown_frame_type() {
        let mut buf = vec![0xFF, 0, 0, 0, 0];
        buf.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(decode(&buf), Err(DecodeError::UnknownFrameType(0xFF)));
    }

    #[test]
    fn invalid_method() {
        // Craft a REQUEST frame with an invalid method byte (0xFF)
        let mut payload = vec![0xFF]; // bad method
        // url: empty
        payload.extend_from_slice(&0u16.to_le_bytes());
        // headers: 0
        payload.extend_from_slice(&0u16.to_le_bytes());
        // body: 0 bytes
        payload.extend_from_slice(&0u32.to_le_bytes());

        let mut buf = vec![FRAME_REQUEST];
        buf.extend_from_slice(&0u32.to_le_bytes()); // request_id
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&payload);

        assert_eq!(decode(&buf), Err(DecodeError::InvalidMethod(0xFF)));
    }

    #[test]
    fn invalid_tunnel_type() {
        let mut payload = vec![0; 16]; // tunnel_id
        payload.extend_from_slice(&[0; 32]); // auth_token
        payload.push(0xFF); // bad tunnel_type

        let mut buf = vec![FRAME_REGISTER];
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&payload);

        assert_eq!(decode(&buf), Err(DecodeError::InvalidTunnelType(0xFF)));
    }

    #[test]
    fn payload_too_short_for_register() {
        // REGISTER requires 49 bytes payload but we only give 10
        let payload = vec![0u8; 10];
        let mut buf = vec![FRAME_REGISTER];
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&payload);

        assert_eq!(decode(&buf), Err(DecodeError::PayloadTooShort));
    }

    #[test]
    fn multiple_frames_in_buffer() {
        let f1 = Frame::Ping { request_id: 10 };
        let f2 = Frame::Pong { request_id: 11 };
        let mut buf = encode(&f1);
        buf.extend_from_slice(&encode(&f2));

        let (decoded1, consumed1) = decode(&buf).unwrap();
        assert_eq!(decoded1, f1);

        let (decoded2, consumed2) = decode(&buf[consumed1..]).unwrap();
        assert_eq!(decoded2, f2);
        assert_eq!(consumed1 + consumed2, buf.len());
    }

    #[test]
    fn all_methods_round_trip() {
        let methods = [
            Method::Get,
            Method::Post,
            Method::Put,
            Method::Delete,
            Method::Patch,
            Method::Head,
            Method::Options,
            Method::Connect,
            Method::Trace,
        ];
        for (i, method) in methods.iter().enumerate() {
            let frame = Frame::Request {
                request_id: i as u32,
                method: *method,
                url: "/test".into(),
                headers: vec![],
                body: vec![],
            };
            round_trip(&frame);
        }
    }

    #[test]
    fn frame_type_accessors() {
        assert_eq!(
            Frame::Ping { request_id: 0 }.frame_type(),
            FRAME_PING
        );
        assert_eq!(
            Frame::Pong { request_id: 0 }.frame_type(),
            FRAME_PONG
        );
        assert_eq!(
            Frame::Error {
                request_id: 0,
                code: 0,
                message: String::new()
            }
            .frame_type(),
            FRAME_ERROR
        );
        assert_eq!(
            Frame::WsOpen {
                request_id: 0,
                ws_id: 1,
                url: "/".into(),
                headers: vec![],
                protocols: vec![]
            }
            .frame_type(),
            FRAME_WS_OPEN
        );
    }

    #[test]
    fn request_id_accessor() {
        let frame = Frame::Request {
            request_id: 12345,
            method: Method::Get,
            url: "/".into(),
            headers: vec![],
            body: vec![],
        };
        assert_eq!(frame.request_id(), 12345);
    }

    #[test]
    fn large_body_round_trip() {
        let body = vec![0xAB; 65536];
        let frame = Frame::Response {
            request_id: 999,
            status: 200,
            headers: vec![],
            body,
        };
        round_trip(&frame);
    }

    #[test]
    fn many_headers_round_trip() {
        let headers: Vec<Header> = (0..100)
            .map(|i| Header {
                name: format!("X-Header-{}", i),
                value: format!("value-{}", i),
            })
            .collect();
        let frame = Frame::Request {
            request_id: 50,
            method: Method::Get,
            url: "/test".into(),
            headers,
            body: vec![],
        };
        round_trip(&frame);
    }

    #[test]
    fn zero_length_payload_frames() {
        // PING and PONG have zero-length payloads
        let ping_bytes = encode(&Frame::Ping { request_id: 0 });
        assert_eq!(ping_bytes.len(), HEADER_SIZE);

        let pong_bytes = encode(&Frame::Pong { request_id: 0 });
        assert_eq!(pong_bytes.len(), HEADER_SIZE);
    }

    #[test]
    fn empty_buffer() {
        assert_eq!(decode(&[]), Err(DecodeError::Incomplete));
    }

    #[test]
    fn stream_start_round_trip() {
        let frame = Frame::StreamStart {
            request_id: 80,
            status: 200,
            headers: vec![
                Header {
                    name: "content-type".into(),
                    value: "text/event-stream".into(),
                },
                Header {
                    name: "cache-control".into(),
                    value: "no-cache".into(),
                },
            ],
        };
        round_trip(&frame);
    }

    #[test]
    fn stream_data_round_trip() {
        let frame = Frame::StreamData {
            request_id: 80,
            data: b"data: {\"event\":\"update\"}\n\n".to_vec(),
        };
        round_trip(&frame);
    }

    #[test]
    fn stream_end_round_trip() {
        round_trip(&Frame::StreamEnd { request_id: 80 });
    }

    #[test]
    fn register_tcp_round_trip() {
        let frame = Frame::Register {
            request_id: 50,
            tunnel_id: [0x03; 16],
            auth_token: [0x04; 32],
            tunnel_type: TunnelType::Tcp,
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_open_round_trip() {
        let frame = Frame::TcpOpen {
            request_id: 90,
            stream_id: 1,
            token: "abc123secret".into(),
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_open_empty_token() {
        let frame = Frame::TcpOpen {
            request_id: 91,
            stream_id: 0,
            token: String::new(),
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_open_ack_round_trip() {
        let frame = Frame::TcpOpenAck {
            request_id: 92,
            stream_id: 42,
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_data_round_trip() {
        let frame = Frame::TcpData {
            request_id: 93,
            stream_id: 42,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_data_empty() {
        let frame = Frame::TcpData {
            request_id: 94,
            stream_id: 1,
            data: vec![],
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_data_large() {
        let frame = Frame::TcpData {
            request_id: 95,
            stream_id: 7,
            data: vec![0xAB; 65536],
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_close_round_trip() {
        let frame = Frame::TcpClose {
            request_id: 96,
            stream_id: 42,
            reason: "connection reset".into(),
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_close_empty_reason() {
        let frame = Frame::TcpClose {
            request_id: 97,
            stream_id: 0,
            reason: String::new(),
        };
        round_trip(&frame);
    }

    #[test]
    fn tcp_frame_type_accessors() {
        assert_eq!(
            Frame::TcpOpen { request_id: 0, stream_id: 0, token: String::new() }.frame_type(),
            FRAME_TCP_OPEN
        );
        assert_eq!(
            Frame::TcpOpenAck { request_id: 0, stream_id: 0 }.frame_type(),
            FRAME_TCP_OPEN_ACK
        );
        assert_eq!(
            Frame::TcpData { request_id: 0, stream_id: 0, data: vec![] }.frame_type(),
            FRAME_TCP_DATA
        );
        assert_eq!(
            Frame::TcpClose { request_id: 0, stream_id: 0, reason: String::new() }.frame_type(),
            FRAME_TCP_CLOSE
        );
    }
}
