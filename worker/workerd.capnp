using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    ( name = "rs-rok",
      worker = .rsRokWorker,
    ),
    ( name = "do-data",
      disk = (
        path = "do-data",
        writable = true,
        allowDotfiles = true,
      ),
    ),
  ],
  sockets = [
    ( name = "http",
      address = "*:8787",
      http = (
        style = host,
      ),
      service = "rs-rok",
    ),
  ],
);

const rsRokWorker :Workerd.Worker = (
  compatibilityDate = "2024-12-30",
  compatibilityFlags = ["nodejs_compat"],

  modules = [
    ( name = "worker",
      esModule = embed "dist/index.js",
    ),
    ( name = "worker.wasm",
      wasm = embed "dist/worker.wasm",
    ),
  ],

  bindings = [
    ( name = "TUNNEL_REGISTRY",
      durableObjectNamespace = "TunnelRegistry",
    ),
    ( name = "MODE_REGISTRY",
      durableObjectNamespace = "ModeRegistry",
    ),
  ],

  durableObjectNamespaces = [
    ( className = "TunnelRegistry",
      uniqueKey = "rs-rok-tunnel-registry",
    ),
    ( className = "ModeRegistry",
      uniqueKey = "rs-rok-mode-registry",
    ),
  ],

  durableObjectStorage = (localDisk = "do-data"),
);