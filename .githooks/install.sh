#!/bin/bash
# Point git at the repo's tracked hook directory.
# Run once after cloning.

set -e
cd "$(dirname "$0")/.."
git config core.hooksPath .githooks
echo "Hooks installed. core.hooksPath = $(git config --get core.hooksPath)"
