# Stable Channels — E2E test runner.
#
# One command builds the regtest backend, the app, preps the device, and runs
# the flows with a live checkmarked scoreboard:
#
#   make ios        full lifecycle on the iOS simulator
#   make android    full lifecycle on the Android emulator
#   make mac        backend + native desktop Mac lifecycle flows
#   make mac-smoke  backend + quick native desktop Mac config check
#   make mac-ui     backend + open native desktop Mac UI on regtest
#   make mac-demo   backend + visible native desktop Mac demo on regtest
#
# NOTE: iOS/Android device tests CANNOT run inside Docker (a simulator needs
# macOS+Xcode; the emulator needs the host). Docker hosts only the backend
# (bitcoin-core, block-explorer, ldk-server, sc-lsp, ldk-node, optional
# lsp-gui); Maestro drives the device from the host. These targets orchestrate
# both halves.
#
# Backend-only helpers: make up / down / clean / logs.
# Run a subset:  make ios FLOWS="01_onboard_lightning 02_btc_to_usd"

SHELL := /usr/bin/env bash
S     := ./scripts
FLOWS ?=
LOG   := .last-run.log

# Every target streams to the terminal AND to $(LOG) so you can watch it live
# from another pane:  tail -f e2e/.last-run.log

.PHONY: ios android mac mac-smoke mac-flows mac-ui mac-demo mac-all up rebuild down clean logs help

ios: ## build + prep the iOS simulator, then run the flows
	@set -o pipefail; { $(S)/backend.sh && $(S)/prepare-ios.sh \
		&& $(S)/run-flows.sh ios $(FLOWS); } 2>&1 | tee $(LOG)

android: ## build + prep the Android emulator, then run the flows
	@set -o pipefail; { $(S)/backend.sh && $(S)/prepare-android.sh \
		&& $(S)/run-flows.sh android $(FLOWS); } 2>&1 | tee $(LOG)

mac: mac-flows ## alias for the native desktop Mac lifecycle flows

mac-smoke: ## start the backend, then run native desktop Mac smoke checks
	@set -o pipefail; { $(S)/backend.sh && $(S)/run-mac-smoke.sh; } 2>&1 | tee $(LOG)

mac-flows: ## start the backend, then run native desktop Mac lifecycle flows
	@set -o pipefail; { $(S)/backend.sh && $(S)/run-mac-flows.sh; } 2>&1 | tee $(LOG)

mac-ui: ## start the backend, then open native desktop Mac UI on regtest
	@set -o pipefail; { $(S)/backend.sh && $(S)/run-mac-ui.sh; } 2>&1 | tee $(LOG)

mac-demo: ## start the backend, then run visible native desktop Mac demo
	@set -o pipefail; { $(S)/backend.sh && $(S)/run-mac-demo.sh; } 2>&1 | tee $(LOG)

mac-all: ## start the backend, then run Mac smoke + lifecycle flows
	@set -o pipefail; { $(S)/backend.sh && $(S)/run-mac-smoke.sh \
		&& $(S)/run-mac-flows.sh; } 2>&1 | tee $(LOG)

up: ## start the docker backend (reuse images) and bootstrap the channel
	@$(S)/backend.sh 2>&1 | tee $(LOG)

rebuild: ## force-rebuild backend images, then start (after LSP/harness source changes)
	@REBUILD=1 $(S)/backend.sh 2>&1 | tee $(LOG)

down: ## stop the docker backend (keeps volumes)
	@cd harness && docker compose --profile gui down --remove-orphans

clean: ## stop the backend AND wipe volumes (fresh chain/channel next time)
	@cd harness && docker compose --profile gui down -v --remove-orphans

logs: ## follow the backend logs
	@cd harness && docker compose --profile gui logs -f

help: ## list targets
	@grep -hE '^[a-z-]+:.*##' $(MAKEFILE_LIST) \
		| sort | awk 'BEGIN{FS=":.*##"} {printf "  \033[36m%-9s\033[0m %s\n", $$1, $$2}'
