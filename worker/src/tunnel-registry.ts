import type { Env } from "./index";
import {
  parseFrame,
  encodeRequestFrame,
  encodeRegisterAckFrame,
  encodePongFrame,
  encodeErrorFrame,
  encodeWsOpenFrame,
  encodeWsDataFrame,
  encodeWsCloseFrame,
  encodeTcpOpenFrame,
  encodeTcpOpenAckFrame,
  encodeTcpDataFrame,
  encodeTcpCloseFrame,
  methodToByte,
  FRAME_REGISTER,
  FRAME_RESPONSE,
  FRAME_PING,
  FRAME_PONG,
  FRAME_ERROR,
  FRAME_WS_DATA,
  FRAME_WS_CLOSE,
  FRAME_STREAM_START,
  FRAME_STREAM_DATA,
  FRAME_STREAM_END,
  FRAME_TCP_OPEN,
  FRAME_TCP_OPEN_ACK,
  FRAME_TCP_DATA,
  FRAME_TCP_CLOSE,
  type ParsedFrame,
} from "./wasm-bridge";

// ---------------------------------------------------------------------------
// Durable Object
// ---------------------------------------------------------------------------

interface PendingRequest {
  resolve: (response: Response) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

type Mode = "root" | "named";

type SocketAttachment =
  | { kind: "cli"; mode: Mode }
  | { kind: "public"; wsId: number }
  | { kind: "tcp"; streamId: number };

// Long-poll endpoints can hold requests for tens of seconds. Keep this higher
// than typical polling intervals to avoid false 504s.
const REQUEST_TIMEOUT_MS = 300_000;

interface PendingStream {
  writer: WritableStreamDefaultWriter<Uint8Array>;
  timer: ReturnType<typeof setTimeout>;
}

export class TunnelRegistry implements DurableObject {
  private state: DurableObjectState;
  private env: Env;
  private cliSocket: WebSocket | null = null;
  private pendingRequests = new Map<number, PendingRequest>();
  private pendingStreams = new Map<number, PendingStream>();
  private nextRequestId = 1;
  private nextPublicSocketId = 1;
  private tcpClients = new Map<number, WebSocket>();
  private publicUrl: string | null = null;
  private tunnelSlug: string | null = null;
  private workerOrigin: string | null = null;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
  }

  /** Load persisted tunnel metadata from storage into instance variables. */
  private async loadState(): Promise<void> {
    if (!this.workerOrigin) {
      this.workerOrigin = (await this.state.storage.get<string>("workerOrigin")) ?? null;
    }
    if (!this.tunnelSlug) {
      this.tunnelSlug = (await this.state.storage.get<string>("tunnelSlug")) ?? null;
    }
  }

  async fetch(request: Request): Promise<Response> {
    await this.loadState();
    const url = new URL(request.url);

    const isWebSocketUpgrade = request.headers.get("Upgrade")?.toLowerCase() === "websocket";

    // WebSocket upgrade from CLI client
    if (isWebSocketUpgrade && url.pathname.startsWith("/__rsrok_cli__/")) {
      return this.handleCliWsUpgrade(request);
    }

    // TCP client WebSocket upgrade (path: /__rsrok_tcp__/<slug>)
    if (isWebSocketUpgrade && url.pathname.startsWith("/__rsrok_tcp__/")) {
      return this.handleTcpClientWsUpgrade(request);
    }

    // Public WebSocket request to forward through tunnel
    if (isWebSocketUpgrade) {
      return this.handlePublicWsUpgrade(request);
    }

    // HTTP request to forward through tunnel
    return this.handleTunnelRequest(request);
  }

  private nextId(): number {
    const id = this.nextRequestId++;
    if (this.nextRequestId > 0xFFFFFFFF) this.nextRequestId = 1;
    return id;
  }

  private toForwardUrl(url: URL): string {
    // Root tunnel forwards full path; named tunnels strip slug prefix.
    if (this.tunnelSlug === "__root__") {
      return url.pathname + url.search;
    }
    const prefix = url.pathname.match(/^\/[a-zA-Z0-9_-]+/)?.[0] ?? "";
    return (url.pathname.slice(prefix.length) || "/") + url.search;
  }

  private getSocketAttachment(ws: WebSocket): SocketAttachment | null {
    try {
      return (ws.deserializeAttachment() as SocketAttachment | null) ?? null;
    } catch {
      return null;
    }
  }

  private setSocketAttachment(ws: WebSocket, attachment: SocketAttachment): void {
    try {
      ws.serializeAttachment(attachment);
    } catch {
      // ignore if attachment serialization fails
    }
  }

  private getCliSocket(): WebSocket | null {
    const [ws] = this.state.getWebSockets("cli");
    if (!ws || ws.readyState !== WebSocket.READY_STATE_OPEN) return null;
    return ws;
  }

  private getPublicSocket(wsId: number): WebSocket | null {
    for (const ws of this.state.getWebSockets("public")) {
      const attachment = this.getSocketAttachment(ws);
      if (attachment?.kind === "public" && attachment.wsId === wsId) {
        return ws;
      }
    }
    return null;
  }

  private async handleCliWsUpgrade(request: Request): Promise<Response> {
    // Only allow one CLI connection per tunnel
    if (this.cliSocket && this.cliSocket.readyState === WebSocket.READY_STATE_OPEN) {
      return new Response("Tunnel already has an active CLI connection", { status: 409 });
    }

    const url = new URL(request.url);
    // Capture and persist the worker's origin and tunnel slug for HTTP request routing
    this.workerOrigin ??= url.origin;
    this.tunnelSlug ??= url.pathname.split("/").at(-1) ?? null;
    await this.state.storage.put("workerOrigin", this.workerOrigin);
    if (this.tunnelSlug) await this.state.storage.put("tunnelSlug", this.tunnelSlug);

    const mode: Mode = this.tunnelSlug === "__root__" ? "root" : "named";

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    this.state.acceptWebSocket(server, ["cli"]);
    this.setSocketAttachment(server, { kind: "cli", mode });
    this.cliSocket = server;

    return new Response(null, { status: 101, webSocket: client });
  }

  private async handleTcpClientWsUpgrade(_request: Request): Promise<Response> {
    const cli = this.getCliSocket();
    if (!cli) {
      return new Response("Tunnel not connected", { status: 502 });
    }

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    // Don't assign streamId yet — the TCP client will send a TCP_OPEN frame
    // with streamId after connecting. We tag with streamId=0 as placeholder.
    this.state.acceptWebSocket(server, ["tcp"]);
    this.setSocketAttachment(server, { kind: "tcp", streamId: 0 });

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }

  private async handlePublicWsUpgrade(request: Request): Promise<Response> {
    const cli = this.getCliSocket();
    if (!cli) {
      return new Response("Tunnel not connected", { status: 502 });
    }

    const url = new URL(request.url);
    const forwardUrl = this.toForwardUrl(url);

    const wsId = this.nextPublicSocketId++;
    if (this.nextPublicSocketId > 0xFFFFFFFF) this.nextPublicSocketId = 1;

    const hopByHop = new Set([
      "connection",
      "keep-alive",
      "transfer-encoding",
      "upgrade",
      "host",
      "sec-websocket-key",
      "sec-websocket-version",
      "sec-websocket-extensions",
      "sec-websocket-protocol",
    ]);
    const headers: [string, string][] = [];
    for (const [name, value] of request.headers) {
      if (!hopByHop.has(name.toLowerCase())) {
        headers.push([name, value]);
      }
    }

    const protocols = (request.headers.get("Sec-WebSocket-Protocol") ?? "")
      .split(",")
      .map((v) => v.trim())
      .filter(Boolean);

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.state.acceptWebSocket(server, ["public"]);
    this.setSocketAttachment(server, { kind: "public", wsId });

    try {
      cli.send(encodeWsOpenFrame(this.nextId(), wsId, forwardUrl, headers, protocols));
    } catch {
      server.close(1011, "Tunnel not connected");
      return new Response("Tunnel not connected", { status: 502 });
    }

    return new Response(null, { status: 101, webSocket: client });
  }

  private async handleTunnelRequest(request: Request): Promise<Response> {
    const ws = this.getCliSocket();
    if (!ws) {
      return new Response("Tunnel not connected", { status: 502 });
    }

    const url = new URL(request.url);
    const forwardUrl = this.toForwardUrl(url);

    const requestId = this.nextId();

    // Collect headers (skip hop-by-hop)
    const hopByHop = new Set(["connection", "keep-alive", "transfer-encoding", "upgrade", "host"]);
    const headers: [string, string][] = [];
    for (const [name, value] of request.headers) {
      if (!hopByHop.has(name.toLowerCase())) {
        headers.push([name, value]);
      }
    }

    // Read body
    const bodyBuf = await request.arrayBuffer();
    const body = new Uint8Array(bodyBuf);

    // Encode and send REQUEST frame via WASM
    const methodByte = methodToByte(request.method);
    const frame = encodeRequestFrame(requestId, methodByte, forwardUrl, headers, body);
    ws.send(frame);

    // Wait for RESPONSE frame from CLI
    return new Promise<Response>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingRequests.delete(requestId);
        resolve(new Response("Tunnel request timed out", { status: 504 }));
      }, REQUEST_TIMEOUT_MS);

      this.pendingRequests.set(requestId, { resolve, reject, timer });
    });
  }

  webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): void {
    const attachment = this.getSocketAttachment(ws);
    if (attachment?.kind === "public") {
      this.handlePublicSocketMessage(attachment.wsId, message);
      return;
    }

    if (attachment?.kind === "tcp") {
      this.handleTcpClientMessage(ws, attachment, message);
      return;
    }

    if (typeof message === "string") return; // binary protocol only from CLI

    // Load state async then handle. webSocketMessage must return void so we use
    // a self-contained async IIFE to avoid blocking the hibernation scheduler.
    void (async () => {
      await this.loadState();
      const data = new Uint8Array(message instanceof ArrayBuffer ? message : (message as any).buffer ?? message);
      const parsed = parseFrame(data);
      if (!parsed) return;

      switch (parsed.frameType) {
        case FRAME_REGISTER:
          this.handleRegister(ws, parsed);
          break;
        case FRAME_RESPONSE:
          this.handleResponse(parsed);
          break;
        case FRAME_PING:
          ws.send(encodePongFrame(parsed.requestId));
          break;
        case FRAME_PONG:
          // heartbeat acknowledged, nothing to do
          break;
        case FRAME_ERROR:
          this.handleError(parsed);
          break;
        case FRAME_WS_DATA:
          this.handleWsData(parsed);
          break;
        case FRAME_WS_CLOSE:
          this.handleWsClose(parsed);
          break;
        case FRAME_STREAM_START:
          this.handleStreamStart(parsed);
          break;
        case FRAME_STREAM_DATA:
          this.handleStreamData(parsed);
          break;
        case FRAME_STREAM_END:
          this.handleStreamEnd(parsed);
          break;
        case FRAME_TCP_OPEN_ACK:
          this.handleTcpOpenAck(parsed);
          break;
        case FRAME_TCP_DATA:
          this.handleTcpDataFromCli(parsed);
          break;
        case FRAME_TCP_CLOSE:
          this.handleTcpCloseFromCli(parsed);
          break;
        default:
          console.error("Unknown frame type:", parsed.frameType);
      }
    })();
  }

  private handlePublicSocketMessage(wsId: number, message: string | ArrayBuffer): void {
    const cli = this.getCliSocket();
    const publicSocket = this.getPublicSocket(wsId);
    if (!publicSocket) return;

    if (!cli) {
      if (publicSocket.readyState === WebSocket.READY_STATE_OPEN) {
        publicSocket.close(1011, "Tunnel not connected");
      }
      return;
    }

    if (typeof message === "string") {
      const payload = new TextEncoder().encode(message);
      cli.send(encodeWsDataFrame(this.nextId(), wsId, false, payload));
      return;
    }

    const payload = new Uint8Array(message);
    cli.send(encodeWsDataFrame(this.nextId(), wsId, true, payload));
  }

  webSocketClose(ws: WebSocket, code: number, reason: string, wasClean: boolean): void {
    const attachment = this.getSocketAttachment(ws);

    if (attachment?.kind === "public") {
      const cli = this.getCliSocket();
      if (cli && cli.readyState === WebSocket.READY_STATE_OPEN) {
        try {
          cli.send(encodeWsCloseFrame(this.nextId(), attachment.wsId, code || 1000, reason ?? ""));
        } catch {
          // ignore close relay errors
        }
      }
      return;
    }

    if (attachment?.kind === "tcp") {
      const streamId = attachment.streamId;
      if (streamId) {
        this.tcpClients.delete(streamId);
        const cli = this.getCliSocket();
        if (cli && cli.readyState === WebSocket.READY_STATE_OPEN) {
          try {
            cli.send(encodeTcpCloseFrame(this.nextId(), streamId, reason ?? "client disconnected"));
          } catch {
            // ignore
          }
        }
      }
      return;
    }

    if (ws === this.cliSocket || attachment?.kind === "cli") {
      this.cliSocket = null;

      // Reject all pending requests
      for (const [id, pending] of this.pendingRequests) {
        clearTimeout(pending.timer);
        pending.resolve(new Response("Tunnel disconnected", { status: 502 }));
      }
      this.pendingRequests.clear();

      // Close all active streams
      for (const [id, stream] of this.pendingStreams) {
        clearTimeout(stream.timer);
        stream.writer.close().catch(() => {});
      }
      this.pendingStreams.clear();

      // Close all active public websocket clients when tunnel disconnects.
      for (const publicWs of this.state.getWebSockets("public")) {
        try {
          publicWs.close(1012, "Tunnel disconnected");
        } catch {
          // ignore
        }
      }

      // Close all active TCP client sockets when tunnel disconnects.
      for (const tcpWs of this.state.getWebSockets("tcp")) {
        try {
          tcpWs.close(1012, "Tunnel disconnected");
        } catch {
          // ignore
        }
      }
      this.tcpClients.clear();

      const mode: Mode = attachment?.kind === "cli"
        ? attachment.mode
        : this.tunnelSlug === "__root__"
        ? "root"
        : "named";

      // Unregister from mode registry so next connection can pick a different mode
      void this.env.MODE_REGISTRY
        .get(this.env.MODE_REGISTRY.idFromName("__singleton__"))
        .fetch(new Request("https://mode-registry/unregister", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ mode }),
        }));
    }
  }

  webSocketError(ws: WebSocket, error: unknown): void {
    console.error("WebSocket error:", error);
    this.webSocketClose(ws, 1011, "WebSocket error", false);
  }

  private handleRegister(ws: WebSocket, frame: ParsedFrame): void {
    const tunnelId = frame.tunnelId ?? new Uint8Array(16);

    // Root tunnel lives at the worker origin root; named tunnels get a path prefix
    const origin = this.workerOrigin ?? "";
    const publicUrl = this.tunnelSlug === "__root__"
      ? origin + "/"
      : `${origin}/${this.tunnelSlug}`;
    this.publicUrl = publicUrl;
    this.cliSocket = ws;

    // Send REGISTER_ACK via WASM
    const ack = encodeRegisterAckFrame(frame.requestId, tunnelId, publicUrl);
    ws.send(ack);
  }

  private handleResponse(frame: ParsedFrame): void {
    const pending = this.pendingRequests.get(frame.requestId);
    if (!pending) return;

    this.pendingRequests.delete(frame.requestId);
    clearTimeout(pending.timer);

    const status = frame.status ?? 200;
    const headers = frame.headers ?? [];
    const body = frame.body ?? new Uint8Array(0);

    const responseHeaders = new Headers();
    for (const [name, value] of headers) {
      responseHeaders.append(name, value);
    }

    pending.resolve(new Response(body.length > 0 ? body : null, {
      status,
      headers: responseHeaders,
    }));
  }

  private handleError(frame: ParsedFrame): void {
    const pending = this.pendingRequests.get(frame.requestId);
    if (!pending) return;

    this.pendingRequests.delete(frame.requestId);
    clearTimeout(pending.timer);

    const code = frame.code ?? 0;
    const message = frame.message ?? "Unknown error";

    pending.resolve(new Response(`Tunnel error ${code}: ${message}`, { status: 502 }));
  }

  private handleWsData(frame: ParsedFrame): void {
    if (typeof frame.wsId !== "number") return;

    const ws = this.getPublicSocket(frame.wsId);
    if (!ws || ws.readyState !== WebSocket.READY_STATE_OPEN) return;

    const data = frame.data ?? frame.body ?? new Uint8Array(0);
    const isBinary = frame.isBinary ?? true;

    if (isBinary) {
      ws.send(data);
    } else {
      ws.send(new TextDecoder().decode(data));
    }
  }

  private handleWsClose(frame: ParsedFrame): void {
    if (typeof frame.wsId !== "number") return;

    const ws = this.getPublicSocket(frame.wsId);
    if (!ws || ws.readyState !== WebSocket.READY_STATE_OPEN) return;

    const code = frame.code ?? 1000;
    const reason = frame.reason ?? frame.message ?? "";
    ws.close(code, reason);
  }

  private handleStreamStart(frame: ParsedFrame): void {
    const pending = this.pendingRequests.get(frame.requestId);
    if (!pending) return;

    this.pendingRequests.delete(frame.requestId);
    clearTimeout(pending.timer);

    const status = frame.status ?? 200;
    const headers = frame.headers ?? [];

    const responseHeaders = new Headers();
    for (const [name, value] of headers) {
      responseHeaders.append(name, value);
    }

    const { readable, writable } = new TransformStream<Uint8Array>();
    const writer = writable.getWriter();

    const streamTimer = setTimeout(() => {
      this.pendingStreams.delete(frame.requestId);
      writer.close().catch(() => {});
    }, REQUEST_TIMEOUT_MS);

    this.pendingStreams.set(frame.requestId, { writer, timer: streamTimer });

    pending.resolve(new Response(readable, {
      status,
      headers: responseHeaders,
    }));
  }

  private handleStreamData(frame: ParsedFrame): void {
    const stream = this.pendingStreams.get(frame.requestId);
    if (!stream) return;

    const data = frame.data ?? frame.body ?? new Uint8Array(0);
    stream.writer.write(data instanceof Uint8Array ? data : new Uint8Array(data)).catch(() => {
      this.pendingStreams.delete(frame.requestId);
      clearTimeout(stream.timer);
    });
  }

  private handleStreamEnd(frame: ParsedFrame): void {
    const stream = this.pendingStreams.get(frame.requestId);
    if (!stream) return;

    this.pendingStreams.delete(frame.requestId);
    clearTimeout(stream.timer);
    stream.writer.close().catch(() => {});
  }

  // -------------------------------------------------------------------------
  // TCP tunneling
  // -------------------------------------------------------------------------

  private getTcpClientSocket(streamId: number): WebSocket | null {
    const ws = this.tcpClients.get(streamId);
    if (ws && ws.readyState === WebSocket.READY_STATE_OPEN) return ws;
    this.tcpClients.delete(streamId);
    return null;
  }

  /** Handle binary messages arriving from a TCP client WebSocket. */
  private handleTcpClientMessage(
    ws: WebSocket,
    attachment: { kind: "tcp"; streamId: number },
    message: string | ArrayBuffer,
  ): void {
    if (typeof message === "string") return; // binary protocol only

    const data = new Uint8Array(message instanceof ArrayBuffer ? message : (message as any).buffer ?? message);
    const parsed = parseFrame(data);
    if (!parsed) return;

    const cli = this.getCliSocket();
    if (!cli) {
      ws.close(1011, "Tunnel not connected");
      return;
    }

    switch (parsed.frameType) {
      case FRAME_TCP_OPEN: {
        const streamId = parsed.streamId ?? 0;
        // Register the stream -> socket mapping
        this.tcpClients.set(streamId, ws);
        this.setSocketAttachment(ws, { kind: "tcp", streamId });
        // Forward TCP_OPEN to CLI server for auth + connection
        cli.send(encodeTcpOpenFrame(parsed.requestId, streamId, parsed.token ?? ""));
        break;
      }
      case FRAME_TCP_DATA: {
        const streamId = parsed.streamId ?? 0;
        // Forward raw data to CLI server
        cli.send(encodeTcpDataFrame(parsed.requestId, streamId, parsed.data ?? new Uint8Array(0)));
        break;
      }
      case FRAME_TCP_CLOSE: {
        const streamId = parsed.streamId ?? 0;
        this.tcpClients.delete(streamId);
        cli.send(encodeTcpCloseFrame(parsed.requestId, streamId, parsed.reason ?? ""));
        break;
      }
      default:
        // Unknown frame from TCP client, ignore
        break;
    }
  }

  /** TCP_OPEN_ACK from CLI -> forward to TCP client socket. */
  private handleTcpOpenAck(frame: ParsedFrame): void {
    const streamId = frame.streamId ?? 0;
    const ws = this.getTcpClientSocket(streamId);
    if (!ws) return;
    ws.send(encodeTcpOpenAckFrame(frame.requestId, streamId));
  }

  /** TCP_DATA from CLI -> forward to TCP client socket. */
  private handleTcpDataFromCli(frame: ParsedFrame): void {
    const streamId = frame.streamId ?? 0;
    const ws = this.getTcpClientSocket(streamId);
    if (!ws) return;
    ws.send(encodeTcpDataFrame(frame.requestId, streamId, frame.data ?? new Uint8Array(0)));
  }

  /** TCP_CLOSE from CLI -> forward to TCP client and clean up. */
  private handleTcpCloseFromCli(frame: ParsedFrame): void {
    const streamId = frame.streamId ?? 0;
    const ws = this.getTcpClientSocket(streamId);
    this.tcpClients.delete(streamId);
    if (!ws) return;
    try {
      ws.send(encodeTcpCloseFrame(frame.requestId, streamId, frame.reason ?? ""));
      ws.close(1000, "Stream closed");
    } catch {
      // ignore
    }
  }
}
