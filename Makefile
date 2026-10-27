.PHONY: check build release test e2e integration wal smoke perf perf-gate perf-save perf-load perf-e2e-gate perf-e2e-save \
       lint fmt fmt-check clean deploy-help toolchain-bump \
       help gate _gate-startup _gate-partials _gate-api _gate-playwright \
       _shard-infra-smoke _shard-routing _shard-htmx _shard-control _shard-maker shards shards-gated shards-report \
       ci ci-full demo stop reset \
       maker \
       prepare check-links

# Prepare dev environment: local uv cache, venv, playwright browsers
prepare: ## set up local dev env (uv venv + playwright chromium)
	UV_CACHE_DIR=$(CURDIR)/tmp/uv-cache \
		uv sync --project rsx-playground
	UV_CACHE_DIR=$(CURDIR)/tmp/uv-cache \
		cd rsx-playground && .venv/bin/playwright install --with-deps chromium

demo: ## turnkey demo: 3 tokens (PENGU/SOL/BTC) trading live + maker
	./rsx-playground/playground doctor
	./rsx-playground/playground demo trio

stop: ## stop all RSX processes (tear down demo/local cluster)
	./rsx-playground/playground stop-all

reset: ## stop all + wipe state (clean slate)
	./rsx-playground/playground reset

# Default target - show help
help: ## Show this help
	@grep -hE '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "} {printf "  \033[36m%-24s\033[0m %s\n", $$1, $$2}'

# ── Release Gates ───────────────────────────────────────────────────
# Hard-ordered gates: each must be green before the next runs.
# Internal stages remain hard-ordered; use `make gate` for the release check.

PYTEST := rsx-playground/.venv/bin/pytest
PY     := rsx-playground/.venv/bin/python3

gate: _gate-playwright ## ordered Playground release check
	@echo "==> All release gates passed."

# Gate 1: server imports cleanly (no startup crash)
_gate-startup:
	@echo "==> [Gate 1] startup/imports"
	cargo build -p rsx-cli
	target/debug/rsx-cli --version
	cd rsx-playground && $(abspath $(PY)) -c "import server; print('ok')"
	@echo "    PASS: rsx-cli runs, server imports cleanly"

# Gate 2: all page routes + HTMX partials return HTTP 200
_gate-partials: _gate-startup
	@echo "==> [Gate 2] routing/partials"
	cd rsx-playground && $(abspath $(PYTEST)) tests/test_htmx_partials.py \
		--tb=short -q
	@echo "    PASS: all HTMX partials HTTP 200"

# Gate 3: API test suite (processes, risk, WAL, orders, edge cases, proxy).
# Excludes stress tests and integration tests requiring live Rust processes.
# Writes tmp/gate-3-report.json; diffs vs prev in tmp/gate-3-diff.json.
_gate-api: _gate-partials
	@echo "==> [Gate 3] API tests"
	cd rsx-playground && $(abspath $(PYTEST)) \
		tests/api_processes_test.py \
		tests/api_risk_test.py \
		tests/api_config_test.py \
		tests/api_gateway_test.py \
		tests/api_matching_test.py \
		tests/api_wal_test.py \
		tests/api_wal_feed_test.py \
		tests/api_rsx_cli_test.py \
		tests/api_logs_metrics_test.py \
		tests/api_verify_test.py \
		tests/api_orders_test.py \
		tests/api_order_glue_test.py \
		tests/test_order_generators.py \
		tests/api_edge_cases_test.py \
		tests/api_proxy_test.py \
		--tb=short -q
	-cd rsx-playground && $(abspath $(PY)) tests/report_diff.py \
		> tmp/gate-3-diff.json 2>&1
	@echo "    PASS: API tests green"

# Gate 4: full Playwright suite — one execution, timestamped JSON+JUnit proof.
# play-full.sh writes artifacts to tmp/play-artifacts/run-<ts>/ and copies
# the canonical report to tmp/play-artifacts/full-run/{report.json,report.xml}.
_gate-playwright: _gate-api
	@echo "==> [Gate 4] Playwright (full run, JSON+JUnit artifacts)"
	cd rsx-playground/tests && bash play-full.sh
	@echo "    PASS: Playwright suite green"

# play-full: standalone full Playwright run (no gate dependencies).
# Writes timestamped artifacts + updates full-run/ canonical.
_play-full:
	@echo "==> [play-full] Playwright full run"
	cd rsx-playground/tests && bash play-full.sh

# ── Playwright Domain Shards ────────────────────────────────────────
# Each shard runs deterministically. play-shard.sh hashes failure
# signatures and blocks re-runs when signature is unchanged and no
# domain files changed (exit 2 = blocked, exit 1 = new failures).
#
# Internal shard recipes back `make shards-gated`. Run a focused Playwright
# spec directly when debugging one browser domain.

SHARD := rsx-playground/tests/play-shard.sh

# Consecutive green infra-smoke runs required before fan-out unlocks
INFRA_SMOKE_STREAK_N := 3

_shard-infra-smoke:
	@bash $(SHARD) infra-smoke

_shard-routing:
	@bash $(SHARD) routing

_shard-htmx:
	@bash $(SHARD) htmx-partials

_shard-control:
	@bash $(SHARD) process-control

_shard-maker:
	@bash $(SHARD) market-maker

shards: _shard-routing _shard-htmx _shard-control _shard-maker
	@echo "==> All shards passed."

# shards-report: run all shards (continue on failure); publish combined
# per-shard pass/fail counts + failing test IDs to
# tmp/play-artifacts/shards-report/report.txt.
shards-report:
	@bash rsx-playground/tests/play-shards-report.sh

# Gated fan-out: run infra-smoke first; only fan out to all product
# shards once infra-smoke has been green >= INFRA_SMOKE_STREAK_N
# consecutive times.  Single-worker validation lane by default.
shards-gated: _shard-infra-smoke
	@STREAK=0; \
	STREAK_FILE=rsx-playground/tmp/play-sig/infra-smoke.streak; \
	if [ -f "$$STREAK_FILE" ]; then \
		STREAK=$$(cat "$$STREAK_FILE"); \
	fi; \
	if [ "$$STREAK" -lt "$(INFRA_SMOKE_STREAK_N)" ]; then \
		echo "==> [shards-gated] infra-smoke streak=$$STREAK/$(INFRA_SMOKE_STREAK_N): fan-out locked"; \
		echo "    Run 'make shards-gated' $(INFRA_SMOKE_STREAK_N) consecutive times to unlock."; \
	else \
		echo "==> [shards-gated] streak=$$STREAK >= $(INFRA_SMOKE_STREAK_N): unlocking full fan-out"; \
		$(MAKE) _shard-routing _shard-htmx _shard-control _shard-maker \
			&& echo "==> All shards passed."; \
	fi

# ── CI Lane ─────────────────────────────────────────────────────────
# Deterministic single-worker acceptance lane.
# Phases run in strict order; fan-out is BLOCKED by default.
#
# make ci       - phases 1-3 then infra-smoke only (no fan-out)
# make ci-full  - phases 1-3 then shards-gated (fan-out after N=3 greens)
#
# The playwright run inherits workers:1 from playwright.config.ts.
# Fan-out to product shards is unlocked only by make ci-full once
# the infra-smoke streak reaches INFRA_SMOKE_STREAK_N.

ci: _gate-api integration _shard-infra-smoke
	@echo "==> [ci] PROGRESS ok + phases 1-3 + integration + infra-smoke passed"
	@echo "    Run 'make ci-full' to unlock product-shard fan-out."

ci-full: _gate-api integration shards-gated
	@echo "==> [ci-full] all acceptance phases passed."

# CI check: no root-absolute href/src in rendered pages.
# Greps Python source templates for bare /path refs.
# External https:// URLs are allowed; // protocol-relative are allowed.
# Then runs the Python test suite that checks all rendered server routes.
_ABS_GREP := grep -En \
	'(href|src|hx-get|hx-post|hx-put|hx-delete|hx-patch|action)=["\x27]/[^/]'

check-links:
	@echo "==> [check-links] server.py + pages.py source templates"
	@if $(_ABS_GREP) \
		rsx-playground/server.py \
		rsx-playground/pages.py; then \
		echo "FAIL: root-absolute link in Python source" >&2; \
		exit 1; \
	fi
	@echo "    PASS: Python source templates clean"
	@echo "==> [check-links] rendered server routes"
	cd rsx-playground && $(abspath $(PYTEST)) \
		tests/test_no_absolute_links.py --tb=short -q
	@echo "    PASS: no root-absolute links in rendered HTML"

# Type check only (fastest feedback, no codegen)
check: ## type-check the workspace (fastest feedback)
	cargo check --workspace

toolchain-bump: ## pin the newest nightly with the pinned components, run the checks
	components=$$(sed -n 's/^components = \[\(.*\)\]/\1/p' rust-toolchain.toml | tr -d '" '); \
		rustup toolchain install nightly --profile minimal --allow-downgrade \
			--no-self-update --component "$$components"
	date=$$(sed -n 's/^date = "\(.*\)"/\1/p' \
		"$$(rustc +nightly --print sysroot)/lib/rustlib/multirust-channel-manifest.toml"); \
		sed -i "s/^channel = .*/channel = \"nightly-$$date\"/" rust-toolchain.toml
	cat rust-toolchain.toml
	$(MAKE) fmt-check lint test integration

# Debug build. Uses the cranelift codegen backend (see .cargo/config.toml).
build: ## debug build the workspace
	cargo build --workspace

# Optimized release build (LLVM).
release:
	cargo build --release --workspace

# Unit tests - lib + integration test binaries (non-ignored).
# Runs every Rust test that does not require Docker/Postgres.
# Ignored tests (testcontainer-gated) run under `make integration`.
test: ## Rust unit + integration tests (no Docker)
	@echo "==> Running Rust unit + integration tests..."
	cargo test --workspace --tests --lib
	@echo ""
	@echo "==> Python API tests skipped from 'make test'"
	@echo "    (Use 'make e2e' to run full integration tests)"

# E2E tests - ALL E2E tests (Rust + ALL API tests + Playwright)
e2e: ## full E2E: Rust + all API + Playwright
	@echo "==> Running Rust E2E tests..."
	cargo test --workspace --test '*' --no-fail-fast \
		--exclude rsx-risk
	@echo "==> Running rsx-risk E2E tests (serial: env-var tests)..."
	cargo test -p rsx-risk --test '*' --no-fail-fast \
		-- --test-threads=1
	@echo ""
	@echo "==> Running all API E2E tests..."
	cd rsx-playground && uv run pytest tests/api_*.py -v --tb=short -x
	@echo ""
	@echo "==> Running Playwright E2E tests..."
	$(MAKE) _play-full

# Integration tests (1-5min) - testcontainers (Postgres).
# Hard-fails if Docker is unavailable so CI cannot pretend
# "0 failures" while skipping every testcontainer-gated test.
# Tests named known_defect_* pin an open BUGS.md entry and fail until it is fixed.
integration: ## testcontainer integration tests (needs Docker)
	@echo "==> [integration] checking Docker daemon..."
	@if ! docker info >/dev/null 2>&1 \
		&& ! sudo -n docker info >/dev/null 2>&1; then \
		echo "FAIL: Docker daemon unreachable; integration tests need testcontainers." >&2; \
		echo "       Start Docker or run on a host with docker access." >&2; \
		exit 1; \
	fi
	@echo "    Docker OK"
	cargo test --workspace --tests \
		-- --ignored --test-threads=1 --skip known_defect_

# WAL correctness tests (<10s)
wal: ## WAL correctness tests (rsx-cast)
	cargo test -p rsx-cast

# Smoke tests (<1min) - deployed systems
smoke: ## smoke tests against a deployed system
	bash scripts/smoke.sh

# Bump UDP socket buffer sizes so the auto-maker can run without
# overrunning the receive queue (default 212 KB is too small).
# Requires sudo; idempotent. Add to /etc/sysctl.d/99-rsx.conf for
# persistence across reboots.
tune-host:
	sudo sysctl -w net.core.rmem_max=26214400
	sudo sysctl -w net.core.wmem_max=26214400
	sudo sysctl -w net.core.rmem_default=26214400
	@echo "net.core.rmem_max/wmem_max set to 25 MB"
	@echo "To persist: echo 'net.core.rmem_max=26214400' | sudo tee /etc/sysctl.d/99-rsx.conf"

# Build the Go market maker (rsx-maker). When present, the playground
# launches this binary; without it, do_maker_start falls back to the
# Python market_maker.py so the demo still comes up.
maker: ## build the Go market maker binary (rsx-maker)
	cd rsx-maker && go build -o rsx-maker .

# ── Local trading: spin up a cluster, then watch it in rsx-glass ──
local: ## start a local cluster with liquidity (tune, dashboard, cluster, maker)
	-$(MAKE) tune-host
	-$(MAKE) maker
	./rsx-playground/playground start
	./rsx-playground/playground start-all minimal
	@sleep 3
	-curl -fsS -X POST 'http://127.0.0.1:49171/api/maker/start?confirm=yes' -H 'x-confirm: yes' >/dev/null
	@echo "-> local cluster up with a live PENGU book. Watch it: RSX_MD_URL=ws://127.0.0.1:8180 go run ./cmd/glass (in rsx-glass/)"

# Print the single-machine production deploy steps. This dev Makefile
# never runs the production deploy —
# deploy/deploy.sh runs ON the target (rsx.krons.cx), by the founder.
deploy-help:
	@bash -n deploy/deploy.sh && echo "deploy/deploy.sh: syntax ok"
	@echo "-> production deploy is manual, on the target host (deploy/README.md):"
	@echo "   1. mount a dedicated volume at /srv/data/rsx/archive"
	@echo "   2. Postgres up as rsx-postgres.service; stage /opt/rsx/env/secret.env (0400)"
	@echo "   3. nginx/caddy TLS -> 127.0.0.1:8080 (gateway) + :8180 (marketdata)"
	@echo "   4. sudo RSX_DEPLOY_HOST=\$$(hostname -f) ./deploy/deploy.sh --apply"
	@echo "-> dry run first (no --apply) prints every action and changes nothing."

# Reproducible end-to-end demo: start minimal cluster, submit one IOC
# order, wait for a fill in the WAL. Exits 0 on success, 1 on timeout.
# Pre: playground server running (./rsx-playground/playground start)
# Post: fills visible in ./tmp/wal/10_active.wal
demo-trade:
	bash scripts/demo-trade.sh

# Performance benchmarks (Rust). timeout: a hung bench must FAIL
# (exit 124), not just read as "slow" (BENCH-NO-TIMEOUT-GATE).
perf: ## Rust Criterion benchmarks
	timeout 600 cargo bench $(if $(PERF_FILTER),--bench $(PERF_FILTER),)

# Criterion regression gate (developer-local, baseline in tmp/)
perf-gate: ## gate Criterion results against the local baseline
	bash scripts/bench-gate.sh

perf-save: ## save the current Criterion baseline
	bash scripts/bench-gate.sh --save-baseline

# Drive the F1 latency probe under load and write measured
# E2E latency distribution into bench-e2e-latest.json.
# Pre: rsx-playground/playground start-all, then start the maker
# (`make maker` builds the Go rsx-maker; the playground launches it,
# falling back to the Python market_maker.py when unbuilt).
# Canonical run: the worst reasonable external order path. Use MODE=long on
# the same topology to measure the extreme tail.
perf-load: ## measure sustained GW→ME→GW latency under load
	bash scripts/latency-publish.sh

# E2E latency regression gate. Drives the sustained load publisher under
# a small N (default 200), compares the resulting e2e_us.p99
# against a sealed reference (bench-reference.json), fails
# if p50 regresses more than THRESHOLD% (default 10).
# specs/2/22-perf-verification.md §4 specifies this gate.
# Pre: cluster up via `./rsx-playground/playground start-all`.
perf-e2e-gate: ## gate GW→ME→GW latency against sealed reference
	bash scripts/bench-gate-e2e.sh

# Snapshot the current measured e2e_us into bench-reference.json.
# Use this only when intentionally accepting a new floor
# (e.g. after a deliberate optimisation). Commit the result.
perf-e2e-save: ## save the GW→ME→GW latency reference
	bash scripts/bench-gate-e2e.sh --save-reference

# Lint — all targets so warnings can't hide in tests/benches.
lint: ## clippy --all-targets, warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

# Format check — default rustfmt is the source of truth (no rustfmt.toml).
fmt-check: ## verify default rustfmt formatting
	cargo fmt --all --check

# Apply formatting.
fmt: ## apply default rustfmt formatting
	cargo fmt --all

# Clean build artifacts
clean: ## remove build artifacts (cargo clean)
	cargo clean
