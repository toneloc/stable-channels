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

The files are bootstrapped only when missing. Operator edits and existing
permissions/ownership are preserved across app restarts and upgrades, with the
missing-field migration below for rebuilt LDK images. Stop the app before
editing over SSH, then start it to apply the changes. Invalid configuration
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

### Detailed forwarded-payment history and version compatibility

LDK Server at `bd95e187b0c08b3fb90fc42a96f0f8a2b6773495` defaults to
`forwarded_payment_tracking_mode = "stats"`. SC's missed-forward reconstruction
needs `"detailed"` under `[node]`; stats totals cannot reconstruct individual
forwards. Detailed history has a limited retention window (about two hours).
Enabling it cannot recover older forwards or forwards recorded only as stats.

The checked-in compose still pins the older `5f631bd` images. Those LDK images
reject this setting, so `hooks/pre-start` deliberately omits it. Merely updating
the hook/package, or restarting an old cached image, does **not** enable detailed
history. Do not manually add this field to an old pinned installation.

The capability gate is **inside the new image**, not a tag, environment flag or
guess about `latest`: `Dockerfile.ldk-server` builds the fixed `bd95e187` revision
and installs `ldk-server-entrypoint.py`. Before starting LDK, that entrypoint:

- Validates the complete UTF-8 TOML, and inserts the missing key into `[node]`
  for both fresh bootstrapped and existing configurations. It reparses the result
  to check that the only semantic change is the new field; comments, secrets,
  unrelated settings and line endings are retained.
- Preserves every explicit setting, including `"stats"` and `"detailed"`.
  Keeping `"stats"` intentionally leaves individual-forward recovery unavailable.
- Atomically replaces only that config file, preserving its owner, group, mode
  and extended attributes/ACLs. Repeated starts do not rewrite an explicit value.
  No seed, wallet/channel database, TLS material or API key is migrated.
- Refuses invalid or unsafe-to-edit layouts, symlinks/hardlinks, and failures to
  write or preserve metadata. LDK is **not started**; the source is left intact
  and logs give a manual migration path without printing configuration values.

For automatic migration, mount the **whole config directory read-write** at
`/etc/ldk-server`, as in this compose file. Container UID/GID `1000:1000` must be
able to read the original and create/rename a sibling file while preserving its
metadata. A read-only directory mount or individual file bind mount cannot be
atomically migrated. SC's separate config directory mount stays read-only.
Existing operator ownership/permissions are no longer recursively reset by the
hook; it assigns container ownership only to newly created paths and unused
package directories containing only `.gitkeep`. Resolve access problems explicitly
rather than broadening private access.

For manual migration, stop the entire app, back up the complete app data
(including configs and permissions), and confirm that **both** daemon images
are the matching rebuilt versions. Validate/fix the TOML and add
`forwarded_payment_tracking_mode = "detailed"` to the existing `[node]` table.
For inline/dotted table layouts or other layouts the migrator refuses, make the
equivalent edit with a TOML-aware editor. Preserve all other values and private
permissions. Once the key is explicit, the entrypoint needs only read access.
Do not edit concurrently with startup or bypass the entrypoint after a failure.

Upgrade the LDK, SC daemon and GUI images together from one source commit;
the LDK build revision and SC client protobuf pin must agree. Package changes
alone are insufficient. Before a downgrade, stop the app, back up its current
data and review upstream state-format compatibility. An older LDK will reject
even an explicit `"stats"` field: remove **only this config field** after that
review, or restore the pre-upgrade config while retaining current channel data.
Never restore stale channel state as a way to undo a config migration.

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

In another terminal, build the current images, publish them to Umbrel Dev's
local registry, and generate the community store:

```bash
cd umbrel/test
./run-community.sh prepare
```

Use `./run-community.sh rebuild` after changing application code or a
Dockerfile. Both `prepare` and `rebuild` build all three images from this checkout
using Docker's layer cache before generating the store; named local images alone
are not evidence of compatibility. The generated compose inherits the writable
LDK config directory and the image's entrypoint. `make-store.sh` only substitutes
image references; invoking it alone neither rebuilds nor validates those images.
Existing Umbrel installs must actually update/recreate their containers to use
newly built images; reusing an old `:local` container leaves old behavior in place.

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

Offline configuration regression tests (Python 3.11+, no Docker/network):

```bash
PYTHONDONTWRITEBYTECODE=1 python3 umbrel/test/test-forwarded-history.py
bash -n umbrel/stable-channels-lsp/hooks/pre-start umbrel/test/run-community.sh
```

These exercise temporary configs and a stub daemon. Release acceptance still
needs real container mounts/UIDs on both target architectures, old-image and
rebuilt-image upgrade/restart checks, graceful shutdown, and a retained forwarded
payment recovered after an SC interruption within LDK's history window.

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
files are never regenerated automatically; only the rebuilt LDK image's guarded
missing-history-field insertion described above can amend an existing config.

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
