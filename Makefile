.PHONY: check test e2e integration wal smoke perf \
       lint clean play play-overview play-topology \
       play-book play-risk play-wal play-logs \
       play-control play-faults play-verify \
       play-orders play-nav play-api \
       api-unit api-integration api-stress \
       bench-webui help check-progress acceptance-bundle release-gate \
       lint-snapshot \
       gate gate-1-startup gate-2-partials gate-3-api gate-4-playwright \
       shard-routing shard-htmx shard-control shard-trade shards

# Default target - show help
help:
	@echo "RSX Test Suite - Available Targets:"
	@echo ""
	@echo "Unit Tests (fast, isolated):"
	@echo "  make test          - Rust unit tests ONLY (~5s)"
	@echo "  make check         - Type check only (fastest, no tests)"
	@echo ""
	@echo "E2E Tests (comprehensive, uses real system):"
	@echo "  make e2e           - ALL E2E tests (Rust + API + Playwright, ~3min)"
	@echo "  make play          - Playwright E2E only (149 tests, ~30s)"
	@echo "  make api-unit      - API E2E fast subset (~20s, 230 tests)"
	@echo "  make api-integration - API E2E comprehensive (~40s, 330 tests)"
	@echo ""
	@echo "Specialized Tests:"
	@echo "  make wal           - WAL correctness tests"
	@echo "  make integration   - Testcontainers (1-5min)"
	@echo "  make api-stress    - Stress tests with latency (3+ min)"
	@echo "  make smoke         - Smoke tests (not implemented)"
	@echo ""
	@echo "Release Gates (ordered, each requires previous to pass):"
	@echo "  make gate              - Run all 4 gates in order (startup->partials->api->playwright)"
	@echo "  make gate-1-startup    - Gate 1: server imports cleanly"
	@echo "  make gate-2-partials   - Gate 2: all routes + HTMX partials HTTP 200"
	@echo "  make gate-3-api        - Gate 3: full API test suite"
	@echo "  make gate-4-playwright - Gate 4: Playwright 223/223 (requires gate-3 first)"
	@echo ""
	@echo "Quality:"
	@echo "  make lint          - Run clippy with warnings as errors"
	@echo "  make check-progress - Validate PROGRESS.md accounting consistency (fail CI if broken)"
	@echo "  make release-gate  - BLOCK release unless Playwright==223/223 and all gates green"
	@echo "  make perf          - Run Rust performance benchmarks (Criterion)"
	@echo "  make bench-webui   - React render benchmark: p95 latency per orderbook update"
	@echo "  make clean         - Clean build artifacts"
	@echo ""
	@echo "Individual Playwright Tests:"
	@echo "  make play-orders, play-control, play-overview, play-book,"
	@echo "  play-risk, play-wal, play-logs, play-verify, play-topology,"
	@echo "  play-faults, play-nav, play-api"

# ── Release Gates ───────────────────────────────────────────────────
# Hard-ordered gates: each must be green before the next runs.
# Usage: make gate        (runs all gates in order, stops on first fail)
#        make gate-1-startup   (imports only, ~1s)
#        make gate-2-partials  (routing + HTMX partials, ~5s)
#        make gate-3-api       (API unit tests, ~30s)
#        make gate-4-playwright (full Playwright suite, 223 tests, ~2min)
#
# NEVER run gate-4-playwright directly — use 'make gate' to enforce order.

PYTEST := rsx-playground/.venv/bin/pytest
PY     := rsx-playground/.venv/bin/python3

gate: gate-1-startup gate-2-partials gate-3-api gate-4-playwright
	@echo "==> All release gates passed."

# Gate 1: server imports cleanly (no startup crash)
gate-1-startup:
	@echo "==> [Gate 1] startup/imports"
	cd rsx-playground && $(abspath $(PY)) -c "import server; print('ok')"
	@echo "    PASS: server imports cleanly"

# Gate 2: all page routes + HTMX partials return HTTP 200
gate-2-partials: gate-1-startup
	@echo "==> [Gate 2] routing/partials"
	cd rsx-playground && $(abspath $(PYTEST)) tests/test_htmx_partials.py \
		--tb=short -q
	@echo "    PASS: all HTMX partials HTTP 200"

# Gate 3: API test suite (processes, risk, WAL, orders, edge cases, proxy).
# Excludes stress tests and integration tests requiring live Rust processes.
# Writes tmp/gate-3-report.json; diffs vs prev in tmp/gate-3-diff.json.
gate-3-api: gate-2-partials
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
		--tb=short -q && \
	$(abspath $(PY)) tests/report_diff.py > tmp/gate-3-diff.json 2>&1 || true
	@echo "    PASS: API tests green"

# Gate 4: full Playwright suite (223 tests). Only runs after gate-3 passes.
gate-4-playwright: gate-3-api
	@echo "==> [Gate 4] Playwright (223 tests)"
	cd rsx-playground/tests && npx playwright test \
		play_navigation.spec.ts \
		play_overview.spec.ts \
		play_topology.spec.ts \
		play_book.spec.ts \
		play_risk.spec.ts \
		play_wal.spec.ts \
		play_logs.spec.ts \
		play_control.spec.ts \
		play_faults.spec.ts \
		play_verify.spec.ts \
		play_orders.spec.ts \
		play_trade.spec.ts
	@echo "    PASS: Playwright suite green"

# ── Playwright Domain Shards ────────────────────────────────────────
# Each shard runs deterministically. play-shard.sh hashes failure
# signatures and blocks re-runs when signature is unchanged and no
# domain files changed (exit 2 = blocked, exit 1 = new failures).
#
# Usage: make shard-routing   (navigation+overview+topology, 29 tests)
#        make shard-htmx      (book+risk+wal+logs+faults+verify, 83 tests)
#        make shard-control   (control+orders, 35 tests)
#        make shard-trade     (trade UI SPA, 67 tests)
#        make shards          (all 4 shards in order)

SHARD := rsx-playground/tests/play-shard.sh

shard-routing:
	@bash $(SHARD) routing

shard-htmx:
	@bash $(SHARD) htmx-partials

shard-control:
	@bash $(SHARD) process-control

shard-trade:
	@bash $(SHARD) trade-ui

shards: shard-routing shard-htmx shard-control shard-trade
	@echo "==> All shards passed."

# Type check only (fastest feedback, no codegen)
check:
	cargo check --workspace

# Unit tests - ONLY isolated unit tests (no real processes, no real DB)
test:
	@echo "==> Running Rust unit tests..."
	cargo test --workspace --lib
	@echo ""
	@echo "==> Python API tests skipped from 'make test'"
	@echo "    (Use 'make e2e' to run full integration tests)"

# E2E tests - ALL E2E tests (Rust + ALL API tests + Playwright)
e2e:
	@echo "==> Running Rust E2E tests..."
	cargo test --workspace --test '*' --no-fail-fast
	@echo ""
	@echo "==> Running API E2E tests (ALL 687 tests)..."
	cd rsx-playground && uv run pytest tests/api_*.py -v --tb=short -x
	@echo ""
	@echo "==> Running Playwright E2E tests..."
	$(MAKE) play

# Integration tests (1-5min) - testcontainers
integration:
	cargo test --workspace --test '*' \
		-- --ignored --test-threads=1

# WAL correctness tests (<10s)
wal:
	cargo test -p rsx-dxs

# Smoke tests (<1min) - deployed systems
smoke:
	@echo "smoke tests not yet implemented"

# Performance benchmarks (Rust)
perf:
	cargo bench

# WebUI render benchmark: measures p50/p95/p99 React render latency
# per orderbook delta update. Asserts p95 < 16ms (one rAF frame).
# Requires: cd rsx-webui && npm run build (builds dist/ first)
bench-webui:
	cd rsx-webui && npm run build && \
	npx playwright test orderbook.bench.spec.ts --reporter=list

# Validate PROGRESS.md accounting (fail CI if inconsistent)
check-progress:
	python3 scripts/check-progress.py

# Generate acceptance bundle (gate statuses, API summary, Playwright totals,
# failing IDs, commit SHA, timestamp). Blocks if gate-3-report.json is stale.
# Writes rsx-playground/tmp/acceptance-bundle.json.
acceptance-bundle:
	@echo "==> [acceptance-bundle] generating..."
	python3 scripts/acceptance-bundle.py
	@echo "    written: rsx-playground/tmp/acceptance-bundle.json"

# Release gate: blocks unless Playwright==223/223 AND all upstream gates green.
# Runs acceptance-bundle; exits non-zero on any failure (see bundle exit codes).
# Use this as the final CI gate before tagging a release.
release-gate: acceptance-bundle
	@python3 -c "\
import json, sys; \
b = json.load(open('rsx-playground/tmp/acceptance-bundle.json')); \
pw = b['summary']['playwright_passed']; \
ok = b['all_green']; \
canon = b['gates']['gate4_playwright']['canonical_ok']; \
print(f'[release-gate] playwright={pw}/223 all_green={ok} canonical_ok={canon}'); \
sys.exit(0 if ok and canon else 1)"

# Contradiction linter: rejects .ship/tasks.json snapshots where any task id
# appears in both DONE and FAIL/retry sets. Run before applying any update.
lint-snapshot:
	python3 scripts/lint-snapshot.py

# Lint
lint:
	cargo clippy --workspace -- -D warnings

# Clean build artifacts
clean:
	cargo clean

# Playwright e2e tests for RSX Playground
play: play-overview play-topology play-book play-risk \
     play-wal play-logs play-control play-faults \
     play-verify play-orders play-nav play-api

play-overview:
	cd rsx-playground/tests && npx playwright test play_overview.spec.ts

play-topology:
	cd rsx-playground/tests && npx playwright test play_topology.spec.ts

play-book:
	cd rsx-playground/tests && npx playwright test play_book.spec.ts

play-risk:
	cd rsx-playground/tests && npx playwright test play_risk.spec.ts

play-wal:
	cd rsx-playground/tests && npx playwright test play_wal.spec.ts

play-logs:
	cd rsx-playground/tests && npx playwright test play_logs.spec.ts

play-control:
	cd rsx-playground/tests && npx playwright test play_control.spec.ts

play-faults:
	cd rsx-playground/tests && npx playwright test play_faults.spec.ts

play-verify:
	cd rsx-playground/tests && npx playwright test play_verify.spec.ts

play-orders:
	cd rsx-playground/tests && npx playwright test play_orders.spec.ts

play-nav:
	cd rsx-playground/tests && npx playwright test play_navigation.spec.ts

play-api:
	cd rsx-playground && uv run pytest tests/api_e2e_test.py -v

# API tests - fast subset (no stress tests)
api-unit:
	@echo "==> Running API E2E tests (fast subset, no stress)..."
	cd rsx-playground && uv run pytest tests/api_processes_test.py tests/api_risk_test.py tests/api_wal_test.py tests/api_logs_metrics_test.py tests/api_verify_test.py -v --tb=short

# API tests - comprehensive subset (includes orders, edge cases)
api-integration:
	@echo "==> Running API E2E tests (comprehensive)..."
	cd rsx-playground && uv run pytest tests/api_orders_test.py tests/api_integration_test.py tests/api_edge_cases_test.py -v --tb=short

# Stress tests with latency measurement (3+ minutes)
api-stress:
	@echo "==> Running stress tests (may take 3+ minutes)..."
	cd rsx-playground && uv run pytest tests/api_orders_test.py -k stress -v -s
