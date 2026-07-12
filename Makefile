.PHONY: check build release test e2e integration wal smoke perf perf-gate perf-save perf-load perf-e2e-gate perf-e2e-save \
       lint fmt fmt-check clean play play-overview play-topology \
       play-book play-risk play-wal play-logs \
       play-control play-faults play-verify \
       play-infra play-orders play-nav play-api \
       play-full deploy-help \
       api-unit api-integration api-stress \
       bench-gate bench-gate-e2e bench-gate-e2e-save bench-save latency-publish help \
       gate _gate-startup _gate-partials _gate-api _gate-playwright gate-1-startup gate-2-partials gate-3-api gate-4-playwright \
       _shard-infra-smoke _shard-routing _shard-htmx _shard-control _shard-maker shard-infra-smoke shard-routing shard-htmx shard-control \
       shard-maker shards shards-gated shards-report \
       ci ci-full demo demo-gif stop reset \
       term term-local term-demo term-smoke-llm term-ssh-setup maker \
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

demo-gif: ## record + render the 3-token demo GIF (run `make demo` first)
	$(MAKE) -C demo gif

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
	cd rsx-playground && $(abspath $(PY)) -c "import server; print('ok')"
	@echo "    PASS: server imports cleanly"

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
		tests/api_wal_test.py \
		tests/api_logs_metrics_test.py \
		tests/api_verify_test.py \
		tests/api_orders_test.py \
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
shards-report: ## run all shards, combined pass/fail report
	@bash rsx-playground/tests/play-shards-report.sh

# Gated fan-out: run infra-smoke first; only fan out to all product
# shards once infra-smoke has been green >= INFRA_SMOKE_STREAK_N
# consecutive times.  Single-worker validation lane by default.
shards-gated: _shard-infra-smoke ## advanced: browser shards after 3 green infra runs
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

ci: _gate-api integration _shard-infra-smoke ## CI lane: API gate + integration + browser smoke
	@echo "==> [ci] PROGRESS ok + phases 1-3 + integration + infra-smoke passed"
	@echo "    Run 'make ci-full' to unlock product-shard fan-out."

ci-full: _gate-api integration shards-gated ## CI lane with gated browser fan-out
	@echo "==> [ci-full] all acceptance phases passed."

# CI check: no root-absolute href/src in dist HTML or rendered pages.
# Greps dist/index.html and Python source templates for bare /path refs.
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
		-- --ignored --test-threads=1

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

# ── Local trading: spin up a cluster, then trade via the TUI ──
local: ## start a local cluster with liquidity (tune, dashboard, cluster, maker)
	-$(MAKE) tune-host
	-$(MAKE) maker
	./rsx-playground/playground start
	./rsx-playground/playground start-all minimal
	@sleep 3
	-curl -fsS -X POST 'http://127.0.0.1:49171/api/maker/start?confirm=yes' -H 'x-confirm: yes' >/dev/null
	@echo "-> local cluster up with a live PENGU book. Trade: make term-local"

# Guarded: the terminal (rsx-term, Go) defaults to wss://rsx.krons.cx
# (production) when RSX_GW_URL is unset. This Makefile is dev-only
# (CLAUDE.md: no external publish/deploy one `make` away) — production
# trading must be an explicit, deliberate command, not a bare `make term`.
# Use term-local/term-demo for dev.
term: ## disabled here — production trading is manual-only, see comment
	@echo "make term is disabled: it would connect to the hosted production" >&2
	@echo "deployment (wss://rsx.krons.cx) by default. This Makefile is" >&2
	@echo "dev-only; production trading must be explicit, not one 'make' away." >&2
	@echo "" >&2
	@echo "Dev alternatives: make term-local (local cluster), make term-demo (mock feed)." >&2
	@echo "To trade against production intentionally, run directly:" >&2
	@echo "  cd rsx-term && RSX_GW_URL=wss://rsx.krons.cx go run ." >&2
	@exit 1

term-local: ## trade against your local cluster (run 'make local' first)
	cd rsx-term && RSX_GW_URL=ws://127.0.0.1:8088 RSX_MD_URL=ws://127.0.0.1:8180 go run .

term-demo: ## the Go terminal offline, mock feed (no cluster needed)
	cd rsx-term && RSX_GW_URL=mock go run .

term-smoke-llm: ## LLM smoke: exercise the assistant live over SOL/ETH/XRP (needs the arizuko stack up)
	@url=$$(.ship/45-ARIZUKO-LLM/chat-token.sh 2>/dev/null | awk -F= '/^RSX_TERM_ASSIST=/{print $$2}'); \
	 [ -n "$$url" ] || { echo "no chat token — is the arizuko stack up? see .ship/45-ARIZUKO-LLM/deploy-local.sh" >&2; exit 1; }; \
	 cd rsx-term && RSX_TERM_ASSIST="$$url" go test ./assistant/ -run TestLLMSmoke -v -count=1 -timeout 600s

term-ssh-setup: ## print SSH forced-command dispatch setup (specs/2/54-tui-access.md)
	@bash -n scripts/rsx-tui-dispatch && bash -n scripts/rsx-tui-authorize \
	  && echo "wrappers: syntax ok"
	@echo "-> install the shared SSH user + wrappers (run as root on the gateway host):"
	@echo "   sudo useradd --system --create-home --shell /usr/sbin/nologin rsx-tui"
	@echo "   sudo install -m 0755 scripts/rsx-tui-dispatch /usr/local/bin/"
	@echo "   sudo install -m 0755 scripts/rsx-tui-authorize /usr/local/bin/"
	@echo "   sudo install -m 0755 target/release/rsx-tui /usr/local/bin/rsx-tui"
	@echo "   sudo install -d -o rsx-tui -g rsx-tui -m 0700 /etc/rsx-tui"
	@echo "   # /etc/rsx-tui/env (mode 0400): RSX_GW_JWT_SECRET=... RSX_GW_URL=wss://rsx.krons.cx"
	@echo "-> register a trader key: rsx-tui-authorize add <user_id> <pubkey> <comment>"
	@echo "   example authorized_keys: scripts/rsx-tui.authorized_keys.example"

# Print the single-machine production deploy steps. Guarded like
# term-ssh-setup: this dev Makefile never runs the production deploy —
# deploy/deploy.sh runs ON the target (rsx.krons.cx), by the founder.
deploy-help: ## print single-machine production deploy steps (deploy/README.md)
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
	timeout 600 cargo bench

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
# Default MODE=single and N=2000. MODE=staircase finds the highest valid
# configured RATES step; MODE=long repeats RATE (or the latest valid rate).
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

# One-release compatibility aliases. Canonical commands are e2e and perf-*.
play play-full:
	@echo "DEPRECATED: use 'make e2e'" >&2
	@$(MAKE) e2e

bench-gate:
	@echo "DEPRECATED: use 'make perf-gate'" >&2
	@$(MAKE) perf-gate

bench-save:
	@echo "DEPRECATED: use 'make perf-save'" >&2
	@$(MAKE) perf-save

latency-publish:
	@echo "DEPRECATED: use 'make perf-load'" >&2
	@$(MAKE) perf-load

bench-gate-e2e:
	@echo "DEPRECATED: use 'make perf-e2e-gate'" >&2
	@$(MAKE) perf-e2e-gate

bench-gate-e2e-save:
	@echo "DEPRECATED: use 'make perf-e2e-save'" >&2
	@$(MAKE) perf-e2e-save

# Focused compatibility aliases print the direct Playwright replacement.

play-infra:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_infra.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_infra.spec.ts

play-overview:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_overview.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_overview.spec.ts

play-topology:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_topology.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_topology.spec.ts

play-book:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_book.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_book.spec.ts

play-risk:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_risk.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_risk.spec.ts

play-wal:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_wal.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_wal.spec.ts

play-logs:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_logs.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_logs.spec.ts

play-control:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_control.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_control.spec.ts

play-faults:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_faults.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_faults.spec.ts

play-verify:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_verify.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_verify.spec.ts

play-orders:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_orders.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_orders.spec.ts

play-nav:
	@echo "DEPRECATED: run 'cd rsx-playground/tests && bunx playwright test play_navigation.spec.ts'" >&2
	cd rsx-playground/tests && bunx playwright test play_navigation.spec.ts

play-api:
	@echo "DEPRECATED: run 'cd rsx-playground && uv run pytest tests/api_e2e_test.py -v'" >&2
	cd rsx-playground && uv run pytest tests/api_e2e_test.py -v

# Former public gate/shard stages remain callable for one release.
gate-1-startup: _gate-startup
gate-2-partials: _gate-partials
gate-3-api: _gate-api
gate-4-playwright: _gate-playwright
shard-infra-smoke: _shard-infra-smoke
shard-routing: _shard-routing
shard-htmx: _shard-htmx
shard-control: _shard-control
shard-maker: _shard-maker

# API tests - fast subset (no stress tests)
api-unit: ## API tests: fast subset (no stress)
	@echo "==> Running API E2E tests (fast subset, no stress)..."
	cd rsx-playground && uv run pytest tests/api_processes_test.py tests/api_risk_test.py tests/api_wal_test.py tests/api_logs_metrics_test.py tests/api_verify_test.py -v --tb=short

# API tests - comprehensive subset (includes orders, edge cases)
api-integration: ## API tests: comprehensive (orders + edge cases)
	@echo "==> Running API E2E tests (comprehensive)..."
	cd rsx-playground && uv run pytest tests/api_orders_test.py tests/api_integration_test.py tests/api_edge_cases_test.py -v --tb=short

# Stress tests with latency measurement (3+ minutes)
api-stress: ## API stress tests with latency (3+ min)
	@echo "==> Running stress tests (may take 3+ minutes)..."
	cd rsx-playground && uv run pytest tests/api_orders_test.py -k stress -v -s
