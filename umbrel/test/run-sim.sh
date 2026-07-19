#!/usr/bin/env bash
# Tier-1 Umbrel simulation driver: fake umbrelOS's injections, run the real
# package hook + production images against the e2e regtest chain.
# Prereq: the e2e backend is up (cd e2e && make up) and the three prod images
# are built (see umbrel/README.md "Publishing images" for the docker build
# commands; local tags sc-{ldk-server,lsp,lsp-gui}-umbrel-test).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

export SIM_APP_DATA_DIR="${SIM_APP_DATA_DIR:-$(pwd)/.sim-app-data}"

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
docker compose -f docker-compose.sim.yml up -d

echo
echo "sim up — verify with:"
echo "  docker logs stable-channels-lsp_ldk-server_1 | grep 'node ID'"
echo "  curl -s http://127.0.0.1:3004/setup | grep -o '[0-9a-f]\{64\}'"
echo "  open http://127.0.0.1:3004"
echo "teardown: docker compose -f docker-compose.sim.yml down && rm -rf $SIM_APP_DATA_DIR"
