# Stable Channels LSP for Umbrel

This package runs an LDK Server Lightning node, the Stable Channels LSP
daemon, and the operator web dashboard against Umbrel's Bitcoin app.

## Architecture

- `ldk-server` owns the Lightning identity, on-chain wallet, channels, and
  LSPS2 JIT-channel service.
- `sc-lsp` owns stable-channel accounting and talks to LDK Server over its
  authenticated TLS gRPC interface.
- `gui` serves the WASM dashboard and proxies same-origin `/api/` requests to
  `sc-lsp`. Umbrel's `app_proxy` protects the dashboard; the GUI container
  only serves `/setup` to requests coming through that proxy.
- Lightning P2P port `19735` is published for wallet connections (`9735` is
  already reserved by Umbrel's LND app). Operators
  still need a reachable IP/domain or Tor endpoint for off-LAN wallets.

The API key shown at `/setup` is an operator secret. It is not a wallet or
provider credential and must never be shared with wallet users.

## Configuration

On first launch, the `pre-start` hook creates these private configuration
files from the installed Bitcoin app's network and RPC exports:

```text
/home/umbrel/umbrel/app-data/stable-channels-lsp/data/config/ldk-server.toml
/home/umbrel/umbrel/app-data/stable-channels-lsp/data/config/sc-lsp.toml
```

The files are bootstrapped only when missing. Operator edits are preserved
across app restarts and upgrades. Edit them over SSH, then stop and start the
Stable Channels LSP app in Umbrel to apply the changes. Invalid configuration
is left intact so the startup error remains available in the app logs.

Mobile push notifications are configured in the optional `[push]` section of
`sc-lsp.toml`. Place `AuthKey.p8` and/or
`firebase-service-account.json` in the app's `data/config` directory and
uncomment the corresponding settings. Missing credentials disable only that
push sender. Device-token registration continues to work.

The LDK Server and SC LSP networks must always match the Bitcoin app. If the
Bitcoin app's network or RPC credentials change, update both files before
restarting, or back them up and remove them to let the hook create fresh
defaults. Never switch networks for an instance that already has funded
channels.

## Local community-store test

Start the official Umbrel development environment:

```bash
git clone --branch 1.7.4 --depth 1 https://github.com/getumbrel/umbrel.git
cd umbrel
npm run dev
```

In Umbrel, install the official Bitcoin Node app first. Open its settings,
change the network to `regtest`, and wait for Bitcoin Node to restart. Do this
before installing Stable Channels LSP because its first-start hook reads the
Bitcoin app's network and creates matching LDK Server and SC LSP
configuration.

In another terminal, build any missing images, publish them to Umbrel Dev's
local registry, and generate the community store:

```bash
cd umbrel/test
./run-community.sh prepare
```

Use `./run-community.sh rebuild` after changing application code or a
Dockerfile.

Keep the store server open in one terminal:

```bash
./run-community.sh serve-store
```

On the default Linux Umbrel Dev network, add the community-store URL printed
by `prepare`: `http://172.17.0.1:8929/stable-channels-app-store/.git`.

This verifies the store, installation hooks, configuration, app lifecycle,
dashboard proxy, and persistence against Umbrel's Bitcoin app. Use regtest
for a safe fully local channel/payment test, or the signet deployment for the
wallet protocol flow. Do not fund an unreviewed local test deployment on
mainnet.

## Image publishing

The `umbrel-images` workflow builds the three multi-architecture images from
`umbrel/docker/` and publishes them to GHCR. The app compose file must pin the
published image digests before release.

Before a public release, publish the same commit's three images for both
`linux/amd64` and `linux/arm64`, pin the multi-architecture digests in
`stable-channels-lsp/docker-compose.yml`, and repeat the acceptance checks on
real amd64 or arm64 Umbrel hardware.

The app manifest uses framework `1.1` because configuration is initialized by
a `pre-start` hook. The hook consumes Umbrel's Bitcoin exports and writes
private, mode-`0600` LDK and SC-LSP configuration files atomically. Existing
files are never regenerated automatically.

The package never deletes or rewrites LDK/LSPS state after a startup error.
A repeated store-read failure remains visible for explicit operator recovery.

## Backups and updates

Umbrel backs up the node seed, Lightning channel databases, and stable-channel
data. Log files are not included. Keep backups current and use the newest
available backup when recovery is necessary.

Before restoring, make sure the original LSP is fully stopped and cannot start
again. Never run the original and restored copies at the same time. Running two
copies of one Lightning node, or restoring old channel state, can force channels
to close and put funds at risk.

Updates must reuse the existing app data directory. Before publishing an
update, install it over an existing test instance and confirm that the node ID,
channels, balances, and stable-channel data remain unchanged after restart.
