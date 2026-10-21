SHELL := /bin/bash
COMPOSE := docker compose

.PHONY: help build image up down restart logs test models weights ext backup weights-check

help:
	@grep -E '^[a-z-]+:.*?##' Makefile | sed 's/:.*##/ —/'

build: ## compile the Rust backend (7G memory scope when systemd is available)
	cd backend && if command -v systemd-run >/dev/null 2>&1; then \
	  systemd-run --user --scope -p MemoryMax=7G --quiet cargo build --release; \
	else cargo build --release; fi

image: ## build the docker image (multi-stage, compiles inside)
	$(COMPOSE) build

up: ## start the service
	@# the bind mount must be owned by the container user, so create it here
	@mkdir -p data
	$(COMPOSE) up -d

down: ## stop the service
	$(COMPOSE) down

restart: ## recreate the container (needed after a .env change)
	$(COMPOSE) up -d --force-recreate

logs: ## follow container logs
	$(COMPOSE) logs -f

test: ## contract test against the running stack (reads .env for credentials)
	@set -a; [ -f .env ] && . ./.env; set +a; \
	  TV_CONTAINER=taste-engine ./scripts/smoke-image.sh

models: ## verify the CLIP ONNX weights are present
	@./scripts/check-models.sh

weights: ## download the CLIP ONNX weights from the release assets
	@./scripts/fetch-weights.sh

weights-check: ## verify the weights are present and match CHECKSUMS.txt
	@./scripts/check-models.sh

ext: ## package both browser extensions into dist/
	@./scripts/pack-extensions.sh

backup: ## sqlite backup + media copy into dist/backups/
	@./scripts/backup.sh
