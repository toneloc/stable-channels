# server/

Cargo workspace for the **Stable Channels LSP** bidaemon architecture, under active development.

## Architecture

```
Clients (lsp-server-gui, iOS wallet, Android wallet)
                │
     REST :3002 (TLS + HMAC, protobuf body)
                ▼
    stable-channels-lsp   (sqlite, audit log, price feed)
                │
       gRPC :3536 (TLS + HMAC)
                ▼
       LDK Server (lightningdevkit/ldk-server, unmodified)
```

`stable-channels-lsp` proxies node level calls to LDK Server over gRPC and serves SC specific data (price feed, stable channel records, audit log) directly.

## Crates

| Crate | Source | Role |
|---|---|---|
| `stable-channels-lsp/` | local | The daemon. axum REST server, HMAC auth, `ldk-server-client` wrapper. |
| `sc-protos/` | local | Hand written `prost` types for SC specific REST endpoints, plus route path constants. |
| `sc-rest-client/` | local | REST client library, linked into the GUI and consumed by mobile wallet apps. WASM compatible. |
| `lsp-server-gui/` | local | Native + WASM egui GUI. Talks to `stable-channels-lsp` over REST. |
| `ldk-server-client` | LDK Server (`lightningdevkit/ldk-server`) | gRPC client used by `stable-channels-lsp` to dial LDK Server. Pinned to upstream via cargo git rev. |
| `ldk-server-grpc` | LDK Server (`lightningdevkit/ldk-server`) | Wire types for LDK Server's gRPC surface (`GetNodeInfoRequest`, `Channel`, etc.). Pulled in transitively via `ldk-server-client`. |
| `stable-channels` (root crate) | local | Shared utility lib (`db`, `audit`, `price_feeds`, `constants`). Path dep'd by the daemon. |

## Build & run

The setup is **three terminals**: LDK Server, the SC daemon, and the GUI. All steps assume `bash`/`zsh`.

### Step 1: Clone the repos

```bash
git clone https://github.com/toneloc/stable-channels.git
git clone https://github.com/lightningdevkit/ldk-server.git
```

The two clones can live anywhere on disk, in any layout. Cargo pulls `ldk-server-client` directly from upstream at a pinned rev, so no sibling-directory layout is required.

### Step 2: Build everything

```bash
cd stable-channels
cargo build --workspace --release

cd /path/to/ldk-server
cargo build --release -p ldk-server
```

### Step 3: Configure LDK Server

Edit LDK Server's `contrib/ldk-server-config.toml`:

- **`[node]`**: set `network = "regtest"` (or `"signet"` / `"bitcoin"` to match your chosen chain backend).
- **Chain source**: leave exactly one of `[bitcoind]` / `[electrum]` / `[esplora]` active and comment the others out. If you don't have a local Bitcoin Core or Electrum server, the easiest path is:
  - Comment out `[bitcoind]` and `[electrum]`.
  - Leave `[esplora]` with `server_url = "https://mempool.space/api"` and set `[node] network = "bitcoin"` (the default esplora endpoint is mainnet).
- **`[tor]`**: uncomment `proxy_address = "127.0.0.1:9050"` (the field must be present in the TOML even if Tor isn't running).
- **`[liquidity.lsps2_client]`**: comment out the entire section (its default placeholder values fail validation).

### Step 4: Run LDK Server (Terminal 1)

```bash
cd /path/to/ldk-server
./target/release/ldk-server ./contrib/ldk-server-config.toml
```

Wait for the log line `gRPC service listening on 127.0.0.1:3536`. LDK Server will auto generate its own `tls.crt`, `tls.key`, and `<network>/api_key` under its `[storage.disk] dir_path`.

### Step 5: Configure the SC daemon (Terminal 2)

```bash
cd ../stable-channels
cp server/stable-channels-lsp/example-config.toml ./sc-config.toml
```

Edit `sc-config.toml`:

- **`[node] network`**: must match LDK Server's network.
- **`[ldk_server] config_path`**: absolute path to LDK Server's config file, e.g.:

  ```toml
  config_path = "/path/to/ldk-server/contrib/ldk-server-config.toml"
  ```

  (The SC daemon reads this to resolve LDK Server's TLS cert path and api_key file.)

### Step 6: Run the SC daemon (Terminal 2)

```bash
./target/release/stable-channels-lsp ./sc-config.toml
```

The daemon auto generates its own `tls.crt`, `tls.key`, and `<network>/api_key` under `./data/stable-channels-lsp/`.

### Step 7: Run the GUI (Terminal 3)

```bash
./target/release/lsp-server-gui
```

### Step 8: Connect

In the GUI window:

1. Click **Load Config**. The GUI auto discovers `sc-config.toml` in the current directory and populates Server URL (`127.0.0.1:3002`), API key (hex of the SC daemon's api_key file), and TLS cert path.
2. Click **Connect**. The status indicator turns green and reads "Connected".
3. **Node Info** tab: real `node_id`, best block hash and height, sync timestamps.
4. **Channels** tab: renders "No channels found." on a fresh node.
5. **Stable** tab: live BTC/USD price, empty stable channels table.
