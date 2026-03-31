// WASM bridge: initializes the Rust WASM module and re-exports
// typed frame encoding/decoding functions for use by the Durable Object.
//
// In Cloudflare Workers (ES modules), importing a .wasm file yields a
// WebAssembly.Module which we pass to initSync() from wasm-bindgen.

import {
  initSync,
  parse_frame,
  encode_request,
  encode_response,
  encode_register_ack,
  encode_ping,
  encode_pong,
  encode_error,
  encode_ws_open,
  encode_ws_data,
  encode_ws_close,
  encode_stream_start,
  encode_stream_data,
  encode_stream_end,
  encode_tcp_open,
  encode_tcp_open_ack,
  encode_tcp_data,
  encode_tcp_close,
} from "../pkg/rs_rok_worker_wasm.js";
import wasmModule from "../pkg/rs_rok_worker_wasm_bg.wasm";

let initialized = false;

function ensureInit() {
  if (!initialized) {
    initSync({ module: wasmModule });
    initialized = true;
  }
}

// Frame type constants (must match protocol crate)
export const FRAME_REGISTER = 0x01;
export const FRAME_REGISTER_ACK = 0x02;
export const FRAME_REQUEST = 0x03;
export const FRAME_RESPONSE = 0x04;
export const FRAME_PING = 0x05;
export const FRAME_PONG = 0x06;
export const FRAME_ERROR = 0x07;
export const FRAME_WS_OPEN = 0x08;
export const FRAME_WS_DATA = 0x09;
export const FRAME_WS_CLOSE = 0x0A;
export const FRAME_STREAM_START = 0x0B;
export const FRAME_STREAM_DATA = 0x0C;
export const FRAME_STREAM_END = 0x0D;
export const FRAME_TCP_OPEN = 0x0E;
export const FRAME_TCP_OPEN_ACK = 0x0F;
export const FRAME_TCP_DATA = 0x10;
export const FRAME_TCP_CLOSE = 0x11;

/** Parsed frame returned by parseFrame() */
export interface ParsedFrame {
  consumed: number;
  frameType: number;
  requestId: number;
  // REGISTER fields
  tunnelId?: Uint8Array;
  authToken?: Uint8Array;
  tunnelType?: number;
  // REGISTER_ACK fields
  publicUrl?: string;
  // REQUEST fields
  method?: number;
  url?: string;
  headers?: [string, string][];
  body?: Uint8Array;
  // RESPONSE fields
  status?: number;
  // ERROR fields
  code?: number;
  message?: string;
  // WS fields
  wsId?: number;
  protocols?: string[];
  isBinary?: boolean;
  data?: Uint8Array;
  reason?: string;
  // TCP fields
  streamId?: number;
  token?: string;
}

/** Parse a binary frame from raw bytes. Returns null if incomplete. */
export function parseFrame(data: Uint8Array): ParsedFrame | null {
  ensureInit();
  try {
    return parse_frame(data) as ParsedFrame;
  } catch {
    return null;
  }
}

/** Encode a REQUEST frame to send to the CLI. */
export function encodeRequestFrame(
  requestId: number,
  method: number,
  url: string,
  headers: [string, string][],
  body: Uint8Array,
): Uint8Array {
  ensureInit();
  const names = headers.map(([n]) => n);
  const values = headers.map(([, v]) => v);
  return encode_request(requestId, method, url, names, values, body);
}

/** Encode a RESPONSE frame. */
export function encodeResponseFrame(
  requestId: number,
  status: number,
  headers: [string, string][],
  body: Uint8Array,
): Uint8Array {
  ensureInit();
  const names = headers.map(([n]) => n);
  const values = headers.map(([, v]) => v);
  return encode_response(requestId, status, names, values, body);
}

/** Encode a REGISTER_ACK frame. */
export function encodeRegisterAckFrame(
  requestId: number,
  tunnelId: Uint8Array,
  publicUrl: string,
): Uint8Array {
  ensureInit();
  return encode_register_ack(requestId, tunnelId, publicUrl);
}

/** Encode a PING frame. */
export function encodePingFrame(requestId: number): Uint8Array {
  ensureInit();
  return encode_ping(requestId);
}

/** Encode a PONG frame. */
export function encodePongFrame(requestId: number): Uint8Array {
  ensureInit();
  return encode_pong(requestId);
}

/** Encode an ERROR frame. */
export function encodeErrorFrame(requestId: number, code: number, message: string): Uint8Array {
  ensureInit();
  return encode_error(requestId, code, message);
}

/** Encode a WS_OPEN frame. */
export function encodeWsOpenFrame(
  requestId: number,
  wsId: number,
  url: string,
  headers: [string, string][],
  protocols: string[],
): Uint8Array {
  ensureInit();
  const names = headers.map(([n]) => n);
  const values = headers.map(([, v]) => v);
  return encode_ws_open(requestId, wsId, url, names, values, protocols);
}

/** Encode a WS_DATA frame. */
export function encodeWsDataFrame(
  requestId: number,
  wsId: number,
  isBinary: boolean,
  data: Uint8Array,
): Uint8Array {
  ensureInit();
  return encode_ws_data(requestId, wsId, isBinary, data);
}

/** Encode a WS_CLOSE frame. */
export function encodeWsCloseFrame(
  requestId: number,
  wsId: number,
  code: number,
  reason: string,
): Uint8Array {
  ensureInit();
  return encode_ws_close(requestId, wsId, code, reason);
}

/** Encode a STREAM_START frame. */
export function encodeStreamStartFrame(
  requestId: number,
  status: number,
  headers: [string, string][],
): Uint8Array {
  ensureInit();
  const names = headers.map(([n]) => n);
  const values = headers.map(([, v]) => v);
  return encode_stream_start(requestId, status, names, values);
}

/** Encode a STREAM_DATA frame. */
export function encodeStreamDataFrame(
  requestId: number,
  data: Uint8Array,
): Uint8Array {
  ensureInit();
  return encode_stream_data(requestId, data);
}

/** Encode a STREAM_END frame. */
export function encodeStreamEndFrame(requestId: number): Uint8Array {
  ensureInit();
  return encode_stream_end(requestId);
}

/** Encode a TCP_OPEN frame. */
export function encodeTcpOpenFrame(
  requestId: number,
  streamId: number,
  token: string,
): Uint8Array {
  ensureInit();
  return encode_tcp_open(requestId, streamId, token);
}

/** Encode a TCP_OPEN_ACK frame. */
export function encodeTcpOpenAckFrame(
  requestId: number,
  streamId: number,
): Uint8Array {
  ensureInit();
  return encode_tcp_open_ack(requestId, streamId);
}

/** Encode a TCP_DATA frame. */
export function encodeTcpDataFrame(
  requestId: number,
  streamId: number,
  data: Uint8Array,
): Uint8Array {
  ensureInit();
  return encode_tcp_data(requestId, streamId, data);
}

/** Encode a TCP_CLOSE frame. */
export function encodeTcpCloseFrame(
  requestId: number,
  streamId: number,
  reason: string,
): Uint8Array {
  ensureInit();
  return encode_tcp_close(requestId, streamId, reason);
}

// Method byte constants (for convenience when building REQUEST frames)
export const METHOD_GET = 0;
export const METHOD_POST = 1;
export const METHOD_PUT = 2;
export const METHOD_DELETE = 3;
export const METHOD_PATCH = 4;
export const METHOD_HEAD = 5;
export const METHOD_OPTIONS = 6;

export function methodToByte(method: string): number {
  switch (method.toUpperCase()) {
    case "GET": return METHOD_GET;
    case "POST": return METHOD_POST;
    case "PUT": return METHOD_PUT;
    case "DELETE": return METHOD_DELETE;
    case "PATCH": return METHOD_PATCH;
    case "HEAD": return METHOD_HEAD;
    case "OPTIONS": return METHOD_OPTIONS;
    default: return METHOD_GET;
  }
}
