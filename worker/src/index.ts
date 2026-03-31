export interface Env {
  TUNNEL_REGISTRY: DurableObjectNamespace;
  MODE_REGISTRY: DurableObjectNamespace;
}

export { TunnelRegistry } from "./tunnel-registry";
export { ModeRegistry } from "./mode-registry";

function modeStub(env: Env) {
  return env.MODE_REGISTRY.get(env.MODE_REGISTRY.idFromName("__singleton__"));
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const url = new URL(request.url);

    // Health check
    if (url.pathname === "/health") {
      return new Response("ok");
    }

    // WebSocket upgrade from CLI: /__rsrok_cli__/:tunnelSlug
    // Keep this on a reserved prefix so root-mode apps can freely use /ws/*.
    const wsMatch = url.pathname.match(/^\/__rsrok_cli__\/([a-zA-Z0-9_-]+)$/);
    if (wsMatch && request.headers.get("Upgrade") === "websocket") {
      const slug = wsMatch[1];
      const requestedMode = slug === "__root__" ? "root" : "named";

      // Check + register mode atomically
      const regResp = await modeStub(env).fetch(
        new Request("https://mode-registry/register", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ mode: requestedMode }),
        })
      );
      if (!regResp.ok) {
        const { error } = await regResp.json<{ error: string }>();
        return new Response(error, { status: 409 });
      }

      const id = env.TUNNEL_REGISTRY.idFromName(slug);
      return env.TUNNEL_REGISTRY.get(id).fetch(request);
    }

    // WebSocket upgrade from TCP client: /__rsrok_tcp__/:tunnelSlug
    const tcpMatch = url.pathname.match(/^\/__rsrok_tcp__\/([a-zA-Z0-9_-]+)$/);
    if (tcpMatch && request.headers.get("Upgrade") === "websocket") {
      const slug = tcpMatch[1];
      const id = env.TUNNEL_REGISTRY.idFromName(slug);
      return env.TUNNEL_REGISTRY.get(id).fetch(request);
    }

    // HTTP proxy — route based on current mode
    const { mode } = await modeStub(env)
      .fetch(new Request("https://mode-registry/mode"))
      .then((r) => r.json<{ mode: "root" | "named" | null }>());

    if (mode === "root") {
      // Forward everything as-is to the root tunnel DO
      return env.TUNNEL_REGISTRY.get(env.TUNNEL_REGISTRY.idFromName("__root__")).fetch(request);
    }

    if (mode === "named") {
      // First path segment is the tunnel name
      const slug = url.pathname.split("/").filter(Boolean)[0];
      if (!slug) return new Response("Not found", { status: 404 });
      return env.TUNNEL_REGISTRY.get(env.TUNNEL_REGISTRY.idFromName(slug)).fetch(request);
    }

    return new Response("No tunnel connected", { status: 502 });
  },
} satisfies ExportedHandler<Env>;
