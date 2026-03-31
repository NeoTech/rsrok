import { describe, it, expect, beforeAll } from "bun:test";
import { initSync, parse_frame, encode_request, encode_response, encode_ping, encode_pong, encode_error, encode_register_ack, encode_tcp_open, encode_tcp_open_ack, encode_tcp_data, encode_tcp_close } from "../pkg/rs_rok_worker_wasm.js";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Frame type constants
const FRAME_REGISTER = 0x01;
const FRAME_REGISTER_ACK = 0x02;
const FRAME_REQUEST = 0x03;
const FRAME_RESPONSE = 0x04;
const FRAME_PING = 0x05;
const FRAME_PONG = 0x06;
const FRAME_ERROR = 0x07;
const FRAME_TCP_OPEN = 0x0E;
const FRAME_TCP_OPEN_ACK = 0x0F;
const FRAME_TCP_DATA = 0x10;
const FRAME_TCP_CLOSE = 0x11;

beforeAll(() => {
  const wasmPath = resolve(__dirname, "../pkg/rs_rok_worker_wasm_bg.wasm");
  const wasmBytes = readFileSync(wasmPath);
  initSync({ module: wasmBytes });
});

describe("WASM bridge", () => {
  it("encodes and parses a REQUEST frame", () => {
    const body = new TextEncoder().encode('{"key":"value"}');
    const frame = encode_request(42, 1, "/api/test?q=1", ["content-type"], ["application/json"], body);
    expect(frame).toBeInstanceOf(Uint8Array);

    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_REQUEST);
    expect(parsed.requestId).toBe(42);
    expect(parsed.method).toBe(1);
    expect(parsed.url).toBe("/api/test?q=1");
    expect(parsed.headers).toHaveLength(1);
    expect(parsed.headers[0]).toEqual(["content-type", "application/json"]);
    expect(new TextDecoder().decode(parsed.body)).toBe('{"key":"value"}');
  });

  it("encodes and parses a RESPONSE frame", () => {
    const body = new TextEncoder().encode("hello");
    const frame = encode_response(99, 200, ["x-custom"], ["test"], body);

    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_RESPONSE);
    expect(parsed.requestId).toBe(99);
    expect(parsed.status).toBe(200);
    expect(parsed.headers).toEqual([["x-custom", "test"]]);
    expect(new TextDecoder().decode(parsed.body)).toBe("hello");
  });

  it("encodes and parses a PING frame", () => {
    const frame = encode_ping(7);
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_PING);
    expect(parsed.requestId).toBe(7);
  });

  it("encodes and parses a PONG frame", () => {
    const frame = encode_pong(8);
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_PONG);
    expect(parsed.requestId).toBe(8);
  });

  it("encodes and parses an ERROR frame", () => {
    const frame = encode_error(5, 502, "Bad Gateway");
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_ERROR);
    expect(parsed.requestId).toBe(5);
    expect(parsed.code).toBe(502);
    expect(parsed.message).toBe("Bad Gateway");
  });

  it("encodes and parses a REGISTER_ACK frame", () => {
    const tunnelId = new Uint8Array(16);
    tunnelId[0] = 0xAB;
    tunnelId[15] = 0xCD;
    const frame = encode_register_ack(10, tunnelId, "/tunnel/abc123");

    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_REGISTER_ACK);
    expect(parsed.requestId).toBe(10);
    expect(parsed.tunnelId[0]).toBe(0xAB);
    expect(parsed.tunnelId[15]).toBe(0xCD);
    expect(parsed.publicUrl).toBe("/tunnel/abc123");
  });

  it("handles empty body request", () => {
    const frame = encode_request(1, 0, "/", [], [], new Uint8Array(0));
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_REQUEST);
    expect(parsed.requestId).toBe(1);
    expect(parsed.method).toBe(0);
    expect(parsed.url).toBe("/");
    expect(parsed.headers).toHaveLength(0);
    expect(parsed.body.length).toBe(0);
  });

  it("handles large request ids", () => {
    const frame = encode_ping(0xFFFFFFFF);
    const parsed = parse_frame(frame);
    expect(parsed.requestId).toBe(0xFFFFFFFF);
  });

  it("WASM and TS encode identical REQUEST bytes", () => {
    // Build the same frame using TS reference and WASM, compare bytes
    const body = new TextEncoder().encode("test");
    const wasmFrame = encode_request(42, 1, "/path", ["host"], ["localhost"], body);

    // Reference TS encoding
    const HEADER_SIZE = 9;
    const urlBytes = new TextEncoder().encode("/path");
    const nameBytes = new TextEncoder().encode("host");
    const valueBytes = new TextEncoder().encode("localhost");
    const payloadLen = 1 + 2 + urlBytes.length + 2 + 2 + nameBytes.length + 2 + valueBytes.length + 4 + body.length;
    const total = HEADER_SIZE + payloadLen;
    const buf = new ArrayBuffer(total);
    const arr = new Uint8Array(buf);
    const dv = new DataView(buf);
    arr[0] = 0x03;
    dv.setUint32(1, 42, true);
    dv.setUint32(5, payloadLen, true);
    let offset = HEADER_SIZE;
    arr[offset++] = 1;
    dv.setUint16(offset, urlBytes.length, true); offset += 2;
    arr.set(urlBytes, offset); offset += urlBytes.length;
    dv.setUint16(offset, 1, true); offset += 2;
    dv.setUint16(offset, nameBytes.length, true); offset += 2;
    arr.set(nameBytes, offset); offset += nameBytes.length;
    dv.setUint16(offset, valueBytes.length, true); offset += 2;
    arr.set(valueBytes, offset); offset += valueBytes.length;
    dv.setUint32(offset, body.length, true); offset += 4;
    arr.set(body, offset);

    expect(Array.from(wasmFrame)).toEqual(Array.from(arr));
  });

  it("encodes and parses a TCP_OPEN frame", () => {
    const frame = encode_tcp_open(50, 1, "my-secret-token");
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_TCP_OPEN);
    expect(parsed.requestId).toBe(50);
    expect(parsed.streamId).toBe(1);
    expect(parsed.token).toBe("my-secret-token");
  });

  it("encodes and parses a TCP_OPEN_ACK frame", () => {
    const frame = encode_tcp_open_ack(51, 2);
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_TCP_OPEN_ACK);
    expect(parsed.requestId).toBe(51);
    expect(parsed.streamId).toBe(2);
  });

  it("encodes and parses a TCP_DATA frame", () => {
    const payload = new TextEncoder().encode("hello tcp");
    const frame = encode_tcp_data(52, 3, payload);
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_TCP_DATA);
    expect(parsed.requestId).toBe(52);
    expect(parsed.streamId).toBe(3);
    expect(new TextDecoder().decode(parsed.data)).toBe("hello tcp");
  });

  it("encodes and parses a TCP_CLOSE frame", () => {
    const frame = encode_tcp_close(53, 4, "connection reset");
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_TCP_CLOSE);
    expect(parsed.requestId).toBe(53);
    expect(parsed.streamId).toBe(4);
    expect(parsed.reason).toBe("connection reset");
  });

  it("encodes and parses a TCP_CLOSE frame with empty reason", () => {
    const frame = encode_tcp_close(54, 5, "");
    const parsed = parse_frame(frame);
    expect(parsed.frameType).toBe(FRAME_TCP_CLOSE);
    expect(parsed.streamId).toBe(5);
    expect(parsed.reason).toBe("");
  });
});
