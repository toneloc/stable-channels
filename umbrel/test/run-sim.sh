#!/usr/bin/env bash
# Tier-1 Umbrel simulation driver: fake umbrelOS's injections, run the real
# package hook + production images against the e2e regtest chain.
# Prereq: the e2e backend is up (cd e2e && make up) and the three prod images
# are built (see umbrel/README.md "Publishing images" for the docker build
# commands; local tags sc-{ldk-server,lsp,lsp-gui}-umbrel-test).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
source ../../e2e/scripts/common.sh

export SIM_APP_DATA_DIR="${SIM_APP_DATA_DIR:-$(pwd)/.sim-app-data}"
compose=(docker compose -f docker-compose.sim.yml)

case "${1:-up}" in
    up) ;;
    down)
        "${compose[@]}" down --remove-orphans
        exit 0
        ;;
    clean|reset)
        "${compose[@]}" down -v --remove-orphans
        rm -rf "$SIM_APP_DATA_DIR" .umbrelos-data
        echo "sim data removed: $SIM_APP_DATA_DIR"
        exit 0
        ;;
    *)
        die "usage: ./run-sim.sh [up|down|clean]"
        ;;
esac

sc_require_free_space "$SIM_APP_DATA_DIR" "${SC_UMBREL_SIM_MIN_FREE_GIB:-25}" "Umbrel sim"
sc_guard_path_size "$SIM_APP_DATA_DIR" "${SC_UMBREL_SIM_MAX_GIB:-12}" "Umbrel sim app data" "SC_UMBREL_SIM_RESET"
sc_guard_path_size "$(pwd)/.umbrelos-data" "${SC_UMBREL_OS_DATA_MAX_GIB:-20}" "UmbrelOS test data" "SC_UMBREL_OS_RESET"
sc_warn_docker_raw_size "${SC_DOCKER_RAW_WARN_GIB:-150}"

# What umbrelOS would inject: APP_DATA_DIR + the Bitcoin app's exports.
# Here they point at the e2e regtest bitcoin-core on the harness network.
APP_DATA_DIR="$SIM_APP_DATA_DIR" \
APP_BITCOIN_NODE_IP="bitcoin-core" \
APP_BITCOIN_RPC_PORT="18443" \
APP_BITCOIN_RPC_USER="sc" \
APP_BITCOIN_RPC_PASS="sc" \
APP_BITCOIN_NETWORK="regtest" \
    bash ../stable-channels-lsp/hooks/pre-start

# The hook renders the SAME container names Umbrel would inject; the sim
# compose pins those with container_name. The sim runs on regtest, so patch
# the one Umbrel-vs-sim delta into the gui env at run time (SC_NETWORK is
# already regtest in docker-compose.sim.yml).
"${compose[@]}" up -d

echo
echo "sim up — verify with:"
echo "  docker logs stable-channels-lsp_ldk-server_1 | grep 'node ID'"
echo "  curl -s http://127.0.0.1:3004/setup | grep -o '[0-9a-f]\{64\}'"
echo "  open http://127.0.0.1:3004"
echo "teardown: ./run-sim.sh down"
echo "wipe sim data: ./run-sim.sh clean"
