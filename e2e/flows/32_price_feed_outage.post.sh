#!/usr/bin/env bash
# Feeds back up (even if the flow died mid-outage), then walk the price home to
# the canonical $100,000 in steps under the oracles' 10% move limit, giving the
# LSP a refresh between steps so each one is accepted.
set -uo pipefail
H=http://localhost:9737
curl -s -X POST $H/feeds/outage -H 'Content-Type: application/json' -d '{"down": false}' > /dev/null
for p in 96000 100000; do
    curl -s -X POST $H/price -H 'Content-Type: application/json' -d "{\"price\": $p.0}" > /dev/null
    sleep 25
done
