# Stable Channels E2E flows (Maestro)

Automated versions of the 12 demo-script user flows, driven by
[Maestro](https://maestro.mobile.dev) against the Android emulator and iOS
simulator. Flow files map 1:1 to the demo-script steps.

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

The harness itself (bitcoind regtest + electrs + LSP bidaemon + counterparty
ldk-node + price mock, docker-compose) is **not yet built** — it is the next
work item. The repo already has the Rust ingredients (`electrsd`,
`corepc-node` dev-deps).

**App-side prerequisite:** the apps currently hardcode mainnet endpoints
(`Constants` on all three platforms: LSP pubkey/address, esplora URL, price
feed URLs). A debug/test build flavor with injectable endpoints is required
before any flow can pass end-to-end.

## Environment variables

Pass with `-e KEY=VALUE`:

| Var | Default | Notes |
|---|---|---|
| `HARNESS_API` | `http://10.0.2.2:9737` | `10.0.2.2` = host loopback from the **Android emulator**. On iOS simulator use `http://localhost:9737`. |
| `RESTORE_SEED` | — | 12-word mnemonic for `11_import_keys` |

## Running

```bash
cd e2e
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
