#!/usr/bin/env bash
# Restore the canonical $100,000 price (a 1% move — inside the oracles' 10% limit).
set -euo pipefail
curl -s -X POST http://localhost:9737/price -H 'Content-Type: application/json' -d '{"price": 100000.0}' > /dev/null
