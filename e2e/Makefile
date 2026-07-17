# Stable Channels — E2E test runner.
#
# One command builds the regtest backend, the app, preps the device, and runs
# the flows with a live checkmarked scoreboard:
#
#   make ios        full lifecycle on the iOS simulator
#   make android    full lifecycle on the Android emulator
#
# NOTE: iOS/Android device tests CANNOT run inside Docker (a simulator needs
# macOS+Xcode; the emulator needs the host). Docker hosts only the backend
# (bitcoind, electrs, ldk-server, sc-lsp, harness); Maestro drives the device
# from the host. These targets orchestrate both halves.
#
# Backend-only helpers: make up / down / clean / logs.
# Run a subset:  make ios FLOWS="01_onboard_lightning 02_btc_to_usd"

SHELL := /usr/bin/env bash
S     := ./scripts
FLOWS ?=
LOG   := .last-run.log

# Every target streams to the terminal AND to $(LOG) so you can watch it live
# from another pane:  tail -f e2e/.last-run.log

.PHONY: ios android up rebuild down clean logs help

ios: ## build + prep the iOS simulator, then run the flows
	@set -o pipefail; { $(S)/backend.sh && $(S)/prepare-ios.sh \
		&& $(S)/run-flows.sh ios $(FLOWS); } 2>&1 | tee $(LOG)

android: ## build + prep the Android emulator, then run the flows
	@set -o pipefail; { $(S)/backend.sh && $(S)/prepare-android.sh \
		&& $(S)/run-flows.sh android $(FLOWS); } 2>&1 | tee $(LOG)

up: ## start the docker backend (reuse images) and bootstrap the channel
	@$(S)/backend.sh 2>&1 | tee $(LOG)

rebuild: ## force-rebuild backend images, then start (after LSP/harness source changes)
	@REBUILD=1 $(S)/backend.sh 2>&1 | tee $(LOG)

down: ## stop the docker backend (keeps volumes)
	@cd harness && docker compose down

clean: ## stop the backend AND wipe volumes (fresh chain/channel next time)
	@cd harness && docker compose down -v

logs: ## follow the backend logs
	@cd harness && docker compose logs -f

help: ## list targets
	@grep -hE '^[a-z-]+:.*##' $(MAKEFILE_LIST) \
		| sort | awk 'BEGIN{FS=":.*##"} {printf "  \033[36m%-9s\033[0m %s\n", $$1, $$2}'
