# E2E regtest harness

One `docker compose up` brings up the full off-app world the Maestro flows
need: regtest chain, esplora, the LSP bidaemon pair, and the harness
(counterparty wallet + miner + price mock) behind the 6-endpoint control API.

## Boot sequence

```bash
cd e2e/harness
docker compose up -d --build          # first build ~15-25 min (two LDK stacks)

# One-time: capture the LSP node id and give it to the harness
docker compose logs ldk-server | grep -i "node id"
cp .env.example .env                  # paste the id into LSP_NODE_ID
docker compose up -d harness          # restart harness with the id

# Fund the counterparty and open its channel to the LSP
curl -X POST localhost:9737/bootstrap -d '{"channel_sats": 5000000}'
curl localhost:9737/info              # sanity: channel_ready: true
```

## Control API (what `e2e/flows/helpers/*.js` call)

| Endpoint | Body | Role |
|---|---|---|
| `POST /pay`     | `{"invoice"}` | counterparty pays; **blocks until settled** (or 120s) |
| `POST /invoice` | `{"amount_msat"}` → `{"invoice"}` | invoice for the app to pay |
| `POST /address` | `{}` → `{"address"}` | counterparty onchain address |
| `POST /send`    | `{"address","amount_sats"}` | counterparty sends onchain |
| `POST /mine`    | `{"blocks"}` | mine regtest blocks |
| `POST /price`   | `{"price"}` | set the mocked BTC/USD price |
| `GET /feeds/{bitstamp,coingecko,kraken,coinbase,blockchain}` | — | price in each real feed's JSON shape |
| `POST /bootstrap` | `{"channel_sats","push_msat"?}` | fund + open channel to LSP |
| `GET /info`     | — | node id, balances, channels, current price |

## What the app under test points at

| App constant | Harness value (from the Android emulator) |
|---|---|
| esplora / chain URL | `http://10.0.2.2:30000` |
| LSP REST | `https://10.0.2.2:3002` (self-signed TLS) |
| LSP pubkey + p2p address | ldk-server's node id @ `10.0.2.2:9735` |
| price feeds (all 5) | `http://10.0.2.2:9737/feeds/<name>` |

(iOS simulator: replace `10.0.2.2` with `localhost`.)

## Verified vs not yet verified

- **Verified:** the harness crate compiles (`cargo check`) against the exact
  ldk-node rev the wallet pins; the ldk-server config keys match
  `contrib/ldk-server-config.toml` at rev `0e4434d`; the sc-lsp config matches
  `server/stable-channels-lsp/example-config.toml`.
- **NOT yet verified:** the docker images build and the five services come up
  and talk to each other (first `docker compose up --build` will tell);
  vulpemventures/electrs flag names; whether ldk-server logs its node id in
  the grep-able form above (fallback: `ldk-server-cli` or the gRPC GetNodeInfo).

## Known gaps (blocking full E2E, tracked in e2e/README.md)

1. **App test flavor** — the wallets hardcode mainnet LSP/esplora/price URLs
   in `Constants`; they need a debug flavor reading the table above.
2. **LSP price injection** — the daemon prices via the shared
   `stable_channels::price_feeds` hardcoded feed list. It needs a small hook
   (env var or config key overriding the feed URLs) to follow `/price`.
   Until then, LSP-side stability behavior tracks the REAL price, not the mock.
3. LSPS2 params in `configs/ldk-server.toml` are loosened for small test
   amounts (1k-sat min vs prod 10k) — revisit if a flow needs prod parity.
