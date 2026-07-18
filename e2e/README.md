# Stable Channels E2E flows (Maestro + native Mac)

Automated versions of the 12 demo-script user flows, driven by
[Maestro](https://maestro.mobile.dev) against the Android emulator and iOS
simulator, plus a native Rust Mac lifecycle runner that uses the same regtest
harness.

## Quickstart

One command builds the regtest backend, builds + installs the app, preps the
device, and runs the full lifecycle with a live checkmarked scoreboard:

```bash
cd e2e
make ios         # full lifecycle on the iOS simulator
make android     # full lifecycle on the Android emulator
make mac         # backend + native desktop Mac lifecycle flows
make mac-smoke   # backend + quick native desktop Mac config check
make mac-ui      # backend + open native desktop Mac UI on regtest
make mac-demo    # backend + visible native desktop Mac demo on regtest
make mac-all     # backend + Mac smoke + lifecycle flows
```

Backend-only helpers, and running a subset of flows:

```bash
make up          # build + start the docker backend, bootstrap the channel
make down        # stop the backend (keeps volumes)
make clean       # stop AND wipe volumes (fresh chain/channel next time)
make logs        # follow backend logs
make help        # list targets

# A subset (state still carries across the listed flows):
make ios FLOWS="01_onboard_lightning 02_btc_to_usd 03_usd_stability"
```

> **Why not `docker compose up ios`?** Device tests can't run inside Docker — a
> simulator needs macOS+Xcode and the emulator needs the host. Docker hosts only
> the backend (bitcoind, electrs, ldk-server, sc-lsp, harness); Maestro drives
> the device from the host. `make ios`/`make android` orchestrate both halves.

Screenshots + logs land in `~/.maestro/tests/<timestamp>/`;
`maestro record <flow>` produces an mp4.

The runner reset-once-then-carry-state model means 01 onboards, 02 trades on
that state, 03 settles, etc. `make ios`/`make android` handle device reset,
so don't add `clearState` to a flow (it wipes the regtest `test_config.json`
and the app silently falls back to MAINNET).

### What the targets do for you

- **backend**: `docker compose up -d --build`, sync the LSP node id into
  `.env`, `POST /bootstrap` (idempotent — funds the LSP + opens the 5M-sat
  channel only if needed), pin the mock price at $100k, and confirm the LSP is
  reading the mock feed (not real ~$64k).
- **iOS**: incremental `xcodebuild` → **verify the override code is actually in
  `StableChannels.debug.dylib`** (guards the stale-incremental-build trap that
  silently ships a MAINNET app) → clean-rebuild only if the check fails → erase
  the selected sim → install → push config before first launch. Set
  `IOS_SIM_UDID` to pin a simulator exactly, or `IOS_SIM_NAME` to select one by
  name; otherwise the runner uses a booted iPhone simulator or the first
  available iPhone simulator.
- **Android**: boot the AVD if needed → `./gradlew installDebug` → `pm clear` +
  push config.
- **Mac desktop**: `make mac-smoke` runs Rust desktop config tests and validates
  local regtest chain/LSP/price-feed configuration before any UI opens.
  `make mac` runs the native Rust lifecycle runner with the same harness roles
  as the mobile flows: counterparty wallet, miner, and mock price feed.
  `make mac-ui` opens the actual Mac app window against that same regtest setup.
  `make mac-demo` opens the Mac app with a progress panel and drives the same
  lifecycle visibly against the regtest harness.

Flows 10/11 (backup/import) are still excluded from the mobile canonical list
because their Maestro navigation is unfinished. The native Mac runner covers
the seed/restart path directly.

## Native Mac

`make mac` uses the same Docker regtest backend as the mobile flows, then runs
`cargo run --features e2e --bin stable-channels -- mac-flows`. It starts from a
fresh dedicated wallet under `e2e/.mac-user-flows` and executes all 12 lifecycle
steps.

`make mac-smoke` runs the fast endpoint/config guard only:
`cargo run --features e2e --bin stable-channels -- mac-smoke`.

`make mac-ui` starts the backend and then opens the real desktop app with:
`cargo run --features e2e --bin stable-channels`. It uses a persistent wallet at
`e2e/.mac-user-ui`; pass `RESET=1` to wipe only that UI wallet before launch:

```bash
make mac-ui
make mac-ui RESET=1
```

`make mac-demo` starts the backend, opens the real desktop app with
`cargo run --features e2e --bin stable-channels -- mac-demo`, and enables a debug-only
`Mac Demo` progress panel. It resets a dedicated wallet at
`e2e/.mac-user-demo` on each run so the visible lifecycle starts cleanly.
The command keeps running after pass/fail until the app window is closed.
The default step pause is 1800ms; override it with
`SC_MAC_DEMO_PAUSE_MS=3000 make mac-demo`.

The Mac runners set Mac-only environment overrides:

- `SC_MAC_NETWORK=regtest`
- `SC_MAC_CHAIN_URL=http://127.0.0.1:30000`
- `SC_MAC_FALLBACK_CHAIN_URL=http://127.0.0.1:30000`
- `SC_MAC_LSP_PUBKEY=<harness LSP node id>`
- `SC_MAC_LSP_ADDRESS=127.0.0.1:9735`
- `SC_MAC_USER_DATA_DIR=e2e/.mac-user` for smoke, `e2e/.mac-user-flows` for flows,
  `e2e/.mac-user-ui` for the UI, and `e2e/.mac-user-demo` for the visible demo

These overrides are ignored unless `SC_E2E=1` is set and the binary is a debug
build. The native Mac path does not replace Maestro coverage for Android/iOS;
it adds a desktop-native guard over the same money-moving lifecycle.

## Status

| Flow | Step | State |
|---|---|---|
| `01_onboard_lightning`  | Onboard over Lightning | runnable after harness (onboarding taps TODO) |
| `02_btc_to_usd`         | BTC → USD | **selectors verified** against SellScreen/HomeScreen |
| `03_usd_stability`      | USD Stability | harness-driven price move; settlement assertion TODO |
| `04_lightning_receive`  | Lightning Receive | **selectors verified** against ReceiveScreen |
| `05_onchain_receive`    | Onchain Receive | selectors verified; needs harness send+mine |
| `06_lightning_send`     | Lightning Send | **selectors verified** against SendScreen |
| `07_onchain_send`       | Onchain Send | selectors verified (asserts "Splice-out initiated") |
| `08_usd_to_btc`         | USD → BTC | **selectors verified** against BuyScreen |
| `09_close_channel`      | Close Channel | Close dialog verified; settings navigation TODO |
| `10_backup_keys`        | Back Up Keys | BackupView labels verified; navigation TODO |
| `11_import_keys`        | Import Keys | Restore labels verified; onboarding TODO |
| `12_offboard_onchain`   | Offboard Onchain | selectors verified ("Send Max") |

"Navigation TODO" = the settings/onboarding tap path needs to be filled in on a
live emulator (`maestro studio` makes this a 2-minute job per flow).

## Prerequisites

1. **Maestro**: `curl -Ls https://get.maestro.mobile.dev | bash`
2. **A booted device**: Android emulator (`emulator -avd <name>`) or iOS
   simulator. Maestro auto-detects whichever is running.
3. **A debug build of the app installed** on the device.
4. **The regtest harness** (see below) — flows that move money need it.
5. **Rust toolchain** for native Mac targets.

## The regtest harness (REQUIRED for money-moving flows)

The flows assume a local test stack exposing one small HTTP control API
("the harness") that plays every off-app role: the counterparty wallet
("another app" in the demo narrative), the miner, and the price feed.

Expected endpoints (see `flows/helpers/*.js`):

| Endpoint | Body | Role |
|---|---|---|
| `POST /pay`       | `{"invoice": "lnbcrt..."}` | counterparty pays our invoice |
| `POST /invoice`   | `{"amount_msat": N}` → `{"invoice": ...}` | counterparty creates an invoice for us to pay |
| `POST /address`   | `{}` → `{"address": "bcrt1..."}` | counterparty onchain address |
| `POST /send`      | `{"address": ..., "amount_sats": N}` | counterparty sends onchain to us |
| `POST /mine`      | `{"blocks": N}` | mine regtest blocks |
| `POST /price`     | `{"price": 100000.0}` | set the mocked BTC/USD price |

The harness itself lives under `e2e/harness/` and is started by the Make
targets via Docker Compose: bitcoind regtest, electrs, ldk-server, sc-lsp, the
counterparty ldk-node, and the mock price feed.

**App-side prerequisite:** mobile debug builds use `test_config.json` overrides.
The Rust Mac desktop wallet uses the `SC_MAC_*` environment overrides described
above, gated by `SC_E2E=1` and debug builds.

## Environment variables

Pass with `-e KEY=VALUE`:

| Var | Default | Notes |
|---|---|---|
| `HARNESS_API` | `http://10.0.2.2:9737` | `10.0.2.2` = host loopback from the **Android emulator**. On iOS simulator use `http://localhost:9737`. |
| `RESTORE_SEED` | — | 12-word mnemonic for `11_import_keys` |

## Running

```bash
cd e2e
make mac                                             # native Mac full lifecycle
make mac-smoke                                       # native Mac config smoke
make mac-ui                                          # open native Mac UI
make mac-demo                                        # watch the native Mac demo
maestro test flows/                                   # full suite, filename order
maestro test flows/02_btc_to_usd.yaml                 # single flow
maestro test --include-tags=trade flows/              # by tag
maestro test --format junit --output report.xml flows/ # CI
maestro record flows/02_btc_to_usd.yaml               # mp4 — compare to demo videos
maestro studio                                        # interactive selector explorer
```

iOS simulator example:

```bash
maestro test -e HARNESS_API=http://localhost:9737 flows/
```

## Canonical test data

Agreed 2026-07-15; every flow and the harness defaults use these. At the base
price, **1,000 sats = $1** — all conversions are checkable by eye.

| Parameter | Value | Why |
|---|---|---|
| Base mock price | **$100,000** | round sats↔USD math |
| Stable target (Steps 1–2) | **$85** | matches prod tester channels (audit `expected_usd` ≈ 85), under the $100 JIT cap |
| Bootstrap channel (counterparty↔LSP) | **5,000,000 sats** | routing headroom for many suite runs |
| Onchain deposit (Step 5) | 100,000 sats (~$100) | |
| Lightning receive (Step 4) | $10 | |
| Lightning send (Step 6) | 5,000 sats ($5) | keep under native so the base run doesn't dip into USD; run variant b) above native for the overflow assertion |
| Onchain send (Step 7) | $5 | |
| Trades (Steps 2/8) | $25 sell, $20 buy-back | buy is capped at the stable position; the sell nets $24.75 after fee |
| Stability move (Step 3) | ±2% → $102,000 / $98,000 | ~$1.70 settlement on the $85 target; clears the $0.25 AND 0.1% thresholds |

## Conventions

- Text selectors are **regex** and must match the on-screen copy exactly —
  a copy change is supposed to break the flow. For long-term durability add
  `Modifier.testTag(...)` (Compose) / `accessibilityIdentifier` (SwiftUI)
  and switch to `id:` selectors; the arrow labels ("USD → BTC") are already
  identical on both platforms.
- Flows are numbered in demo-script order and are written to be **runnable as
  one lifecycle**: onboard → trade → receive → send → trade back → close →
  restore → offboard.
- Every flow carries platform + category `tags`.
