#!/bin/sh
# Self-healing wrapper for ldk-server: defuses the LSPS2 persisted-state
# restart landmine (see explore-lsp-restart-issue.txt at the repo root)
# without patching upstream.
#
# ldk-node (rev f2e44fd, liquidity/mod.rs:283) swallows LiquidityManager::new
# errors into a bare "Failed to build LDK Node: Failed to read from store."
# — persisted per-peer LSPS2 state written during JIT+splice sequences can
# fail to re-deserialize and brick startup forever. Every OTHER cause of that
# BuildError logs its own specific message first, so N consecutive fast exits
# with the bare signature and no companion detail means the LSPS2 reload.
#
# Recovery: back up the store, drop the lightning_liquidity_state namespace
# (service-side JIT bookkeeping only — channels/funds live elsewhere; a
# client mid-JIT-onboard simply retries), and start again.
#
# Usage: ldk-server-guard.sh <config-path> <storage-dir> [network-dir]
#   network-dir defaults to the `network = "..."` value in the config
#   (mainnet -> bitcoin).
set -u

CONFIG="${1:?config path}"
STORAGE="${2:?storage dir}"
NETWORK="${3:-$(sed -n 's/^network *= *"\(.*\)".*/\1/p' "$CONFIG" | head -1)}"
[ "$NETWORK" = "mainnet" ] && NETWORK=bitcoin
NETWORK="${NETWORK:-bitcoin}"
DB="${STORAGE}/${NETWORK}/ldk_node_data.sqlite"
LOG="${STORAGE}/${NETWORK}/ldk-server.log"
SIGNATURE="Failed to build LDK Node: Failed to read from store."
THRESHOLD=3

consecutive=0
while :; do
    start_epoch="$(date +%s)"
    /usr/local/bin/ldk-server "$CONFIG"
    rc=$?
    ran_for=$(( $(date +%s) - start_epoch ))

    # A long-lived run that eventually exits is a normal crash/stop, not the
    # startup landmine — reset the counter.
    if [ "$ran_for" -gt 60 ]; then
        consecutive=0
    elif tail -n 5 "$LOG" 2>/dev/null | grep -qF "$SIGNATURE"; then
        consecutive=$((consecutive + 1))
        echo "[guard] startup ReadFailed signature ($consecutive/$THRESHOLD)" >&2
    else
        consecutive=0
    fi

    if [ "$consecutive" -ge "$THRESHOLD" ] && [ -f "$DB" ]; then
        stamp="$(date +%Y%m%dT%H%M%S)"
        echo "[guard] $THRESHOLD consecutive store-read failures — quarantining LSPS2 state" >&2
        cp "$DB" "${DB}.bak-lsps2-${stamp}" || { echo "[guard] backup failed; NOT touching store" >&2; sleep 30; continue; }
        deleted="$(sqlite3 "$DB" "DELETE FROM ldk_node_data WHERE primary_namespace='lightning_liquidity_state'; SELECT changes();")" \
            || { echo "[guard] sqlite delete failed; store untouched beyond backup" >&2; sleep 30; continue; }
        echo "[guard] removed $deleted lightning_liquidity_state rows (backup: ${DB}.bak-lsps2-${stamp})" >&2
        consecutive=0
    fi

    sleep 5
done
