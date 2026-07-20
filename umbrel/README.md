# Umbrel package — Stable Channels LSP

Runs the LSP bidaemon pair (ldk-server + stable-channels-lsp) plus the web
dashboard as an Umbrel app, against the Umbrel Bitcoin app's node.

```
umbrel/
  docker/                      production Dockerfiles (published multi-arch via
                               .github/workflows/umbrel-images.yml)
    Dockerfile.ldk-server      upstream ldk-server @ 0e4434d, LSPS2 feature on
    Dockerfile.sc-lsp          this repo's daemon, WITHOUT the e2e feature
    Dockerfile.lsp-gui         wasm dashboard + nginx /api/ proxy (env-templated)
  stable-channels-lsp/         the Umbrel app package (manifest + compose + hook)
```

## How it fits together

- `hooks/pre-start` renders `ldk-server.toml` / `sc-lsp.toml` into
  `${APP_DATA_DIR}/data/config` from the Bitcoin app's exports
  (`APP_BITCOIN_NODE_IP`, RPC credentials, network). Umbrel's injected
  container names (`stable-channels-lsp_<service>_1`) are baked into the gRPC
  target and the TLS SAN — same gotcha class as the e2e stack.
- The GUI container proxies `/api/` to sc-lsp internally (the wasm client
  issues origin-relative requests), so the browser never needs the self-signed
  cert and there is no CORS. `app_proxy` fronts the GUI on manifest port 3003.
- First-run: open the app, go to `/setup` (gated by Umbrel login) to copy the
  auto-generated API key into the dashboard's connection screen.
- Lightning P2P (9735) is host-published so wallet users' nodes can reach the
  LSP for JIT opens.

## Publishing images

`gh workflow run umbrel-images` (or push an `umbrel-v*` tag). Then update the
three image tags in `stable-channels-lsp/docker-compose.yml` to the new short
SHA — for App Store submission, pin by digest instead
(`docker buildx imagetools inspect ghcr.io/toneloc/<name>:<sha>`).

## Testing (three tiers, fastest first)

**Tier 1 — local simulation on a Mac (no umbrelOS).** Fake what umbrelOS
injects and run the actual package against the e2e regtest chain: create a
scratch `APP_DATA_DIR`, run `hooks/pre-start` with `APP_BITCOIN_*` env
pointing at the e2e `bitcoin-core` container (network `regtest`), then start
the three production images with Umbrel-style container names
(`stable-channels-lsp_<service>_1`) on the e2e docker network, GUI on a spare
host port in place of `app_proxy`. Exercises hook rendering, the prod images,
the container-name gRPC/TLS-SAN wiring, `/setup`, and the nginx proxy —
everything except Umbrel's manifest parsing and auth gate.

**Tier 2 — real umbrelOS in Docker.** Umbrel publishes an `umbrelos` image
(ghcr.io/getumbrel/umbrelos, used by their dev environment) that runs the
full OS under Docker Desktop. Push this package dir to a repo shaped as a
Community App Store, add it in umbrelOS Settings, install the Bitcoin app,
then this app — the genuine install flow including `app_proxy` auth and
injected container names. Requires the images below to be PUBLISHED to GHCR
(local-only images won't install), and a syncable chain for the Bitcoin app
(check its advanced settings for signet; mainnet sync in a test env is a
non-starter).

**Tier 3 — umbrelOS on real hardware / VM.** Official images are amd64
(`umbrelos-amd64.img.xz` from getumbrel/umbrel releases; wiki has Linux-VM
and x86 guides). On Apple Silicon that means slow QEMU emulation via UTM —
prefer a spare x86 box (bare or Proxmox), which doubles as the long-run soak
test for restart behavior before any store submission.

Verify at every tier: pre-start rendered configs in
`${APP_DATA_DIR}/data/config`; ldk-server logs its node id; `/setup` shows
the key; dashboard connects; 9735 reachable from off-box.

Local sim storage guardrails: `umbrel/test/run-sim.sh` refuses to start with
less than 25 GiB free, stops if `.sim-app-data` exceeds 12 GiB, and stops if
old `.umbrelos-data` exceeds 20 GiB. Use `SC_UMBREL_SIM_MIN_FREE_GIB`,
`SC_UMBREL_SIM_MAX_GIB`, or `SC_UMBREL_OS_DATA_MAX_GIB` to tune those. To wipe
local sim state deliberately, run `cd umbrel/test && ./run-sim.sh clean`, or
set `SC_UMBREL_SIM_RESET=1` / `SC_UMBREL_OS_RESET=1` on the next run.
Container stdout logs are capped at 10 MiB x 3 files, and the ldk-server file
log is trimmed after 50 MiB.

## Known blockers / caveats before real users

- **LSPS2 restart landmine (UPSTREAM, MUST FIX FIRST):** persisted LSPS2
  service state can fail to reload and brick node startup with an unlogged
  "Failed to read from store." — see `explore-lsp-restart-issue.txt` at the
  repo root. Umbrel boxes restart constantly; do not ship to real users until
  the upstream fix (or a carried patch) lands in the ldk-server image.
- **Backups:** `backupIgnore` excludes the channel-state sqlite databases on
  purpose — restoring stale channel state onto live channels is the
  force-close-by-rollback scenario. The seed (`data/ldk-server/keys_mnemonic`)
  IS backed up; recovery is seed-based (see fc-recovery runbooks).
- **No channel announcements configured** — JIT channels are unannounced, so
  the LSP works as a private routing endpoint; add an announcement address to
  the rendered config if/when a public node identity is wanted.
- Manifest `port: 3003` must be unique across the target app store; adjust at
  submission time. `submission:` URL is a TODO placeholder.
