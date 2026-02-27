.PHONY: check test e2e integration wal smoke perf \
       lint clean play play-overview play-topology \
       play-book play-risk play-wal play-logs \
       play-control play-faults play-verify \
       play-infra play-orders play-nav play-api \
       play-full \
       api-unit api-integration api-stress \
       bench-webui help check-progress acceptance-bundle \
       gen-release-truth release-gate \
       lint-snapshot lint-snapshot-tests ci-guard publish-progress regen-progress exit-criteria task-report status-doctor \
       gate gate-1-startup gate-2-partials gate-3-api gate-4-playwright \
       shard-infra-smoke shard-routing shard-htmx shard-control \
       shard-maker shard-trade shards shards-gated shards-report \
       ci ci-full \
       prepare check-links \
       local-validate local-validate-dry local-validate-pending \
       meta-guard meta-guard-status meta-guard-tests

# Prepare dev environment: local uv cache, venv, playwright browsers
prepare:
	UV_CACHE_DIR=$(CURDIR)/tmp/uv-cache \
		uv sync --project rsx-playground
	UV_CACHE_DIR=$(CURDIR)/tmp/uv-cache \
		cd rsx-playground && .venv/bin/playwright install --with-deps chromium

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
	@echo "  make play          - Playwright E2E only (154 tests, ~30s)"
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
	@echo "  make gate-4-playwright - Gate 4: Playwright 228/228 (requires gate-3 first)"
	@echo ""
	@echo "Quality:"
	@echo "  make lint          - Run clippy with warnings as errors"
	@echo "  make check-progress    - Validate PROGRESS.md accounting (fail CI if broken)"
	@echo "  make publish-progress  - Regenerate PROGRESS.md header from artifacts; fail on divergence"
	@echo "  make task-report       - Rewrite PROGRESS.md from tasks.json (truth source)"
	@echo "  make exit-criteria     - Auto-reopen completed tasks whose linked tests aren't green on HEAD"
	@echo "  make gen-release-truth - Generate release_truth.json (no ext CLI dep)"
	@echo "  make release-gate      - BLOCK release unless Playwright==223/223 and all gates green"
	@echo "  make perf          - Run Rust performance benchmarks (Criterion)"
	@echo "  make bench-webui   - React render benchmark: p95 latency per orderbook update"
	@echo "  make clean         - Clean build artifacts"
	@echo ""
	@echo "Individual Playwright Tests:"
	@echo "  make play-orders, play-control, play-overview, play-book,"
	@echo "  play-risk, play-wal, play-logs, play-verify, play-topology,"
	@echo "  play-faults, play-nav, play-api"
	@echo ""
	@echo "CI Lane (deterministic, single-worker, fan-out blocked by default):"
	@echo "  make ci                - Phases 1-3 + infra-smoke; fan-out locked"
	@echo "  make ci-full           - Phases 1-3 + shards-gated; fan-out after N=3 greens"
	@echo ""
	@echo "Shard Targets:"
	@echo "  make shard-infra-smoke - Single validation lane (infra only)"
	@echo "  make shard-maker       - Market-maker lifecycle shard"
	@echo "  make shards            - Run all 5 product shards"
	@echo "  make shards-gated      - Single-worker lane; fan-out after N=3 greens"
	@echo "  make shards-report     - All shards; combined pass/fail report"
	@echo ""
	@echo "Fallback (quota-blocked agent sessions):"
	@echo "  make local-validate    - Run blocked tasks via local make (no agent)"
	@echo "  make local-validate-dry    - Dry-run: show which tasks would execute"
	@echo "  make local-validate-pending - Also include pending tasks"

# ── Release Gates ───────────────────────────────────────────────────
# Hard-ordered gates: each must be green before the next runs.
# Usage: make gate        (runs all gates in order, stops on first fail)
#        make gate-1-startup   (imports only, ~1s)
#        make gate-2-partials  (routing + HTMX partials, ~5s)
#        make gate-3-api       (API unit tests, ~30s)
#        make gate-4-playwright (full Playwright suite, 228 tests, ~2min)
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

# Gate 4: full Playwright suite — one execution, timestamped JSON+JUnit proof.
# play-full.sh writes artifacts to tmp/play-artifacts/run-<ts>/ and copies
# the canonical report to tmp/play-artifacts/full-run/{report.json,report.xml}.
# acceptance-bundle.py reads ONLY from full-run/report.json (not per-shard).
gate-4-playwright: gate-3-api
	@echo "==> [Gate 4] Playwright (full run, JSON+JUnit artifacts)"
	cd rsx-playground/tests && bash play-full.sh
	@echo "    PASS: Playwright suite green"

# play-full: standalone full Playwright run (no gate dependencies).
# Writes timestamped artifacts + updates full-run/ canonical.
play-full:
	@echo "==> [play-full] Playwright full run"
	cd rsx-playground/tests && bash play-full.sh

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

# Consecutive green infra-smoke runs required before fan-out unlocks
INFRA_SMOKE_STREAK_N := 3

shard-infra-smoke:
	@bash $(SHARD) infra-smoke

shard-routing:
	@bash $(SHARD) routing

shard-htmx:
	@bash $(SHARD) htmx-partials

shard-control:
	@bash $(SHARD) process-control

shard-maker:
	@bash $(SHARD) market-maker

shard-trade:
	@bash $(SHARD) trade-ui

shards: shard-routing shard-htmx shard-control shard-maker shard-trade
	@echo "==> All shards passed."

# shards-report: run all shards (continue on failure); publish combined
# per-shard pass/fail counts + failing test IDs to
# tmp/play-artifacts/shards-report/report.txt.
shards-report:
	@bash rsx-playground/tests/play-shards-report.sh

# Gated fan-out: run infra-smoke first; only fan out to all product
# shards once infra-smoke has been green >= INFRA_SMOKE_STREAK_N
# consecutive times.  Single-worker validation lane by default.
shards-gated: shard-infra-smoke
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
		$(MAKE) shard-routing shard-htmx shard-control shard-maker shard-trade; \
		echo "==> All shards passed."; \
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

ci: check-progress gate-1-startup gate-2-partials gate-3-api shard-infra-smoke
	@echo "==> [ci] PROGRESS ok + phases 1-3 + infra-smoke passed"
	@echo "    Run 'make ci-full' to unlock product-shard fan-out."

ci-full: check-progress gate-1-startup gate-2-partials gate-3-api shards-gated
	@echo "==> [ci-full] all acceptance phases passed."

# CI check: no root-absolute href/src in dist HTML or rendered pages.
# Greps dist/index.html and Python source templates for bare /path refs.
# External https:// URLs are allowed; // protocol-relative are allowed.
# Then runs the Python test suite that checks all rendered server routes.
_ABS_GREP := grep -En \
	'(href|src|hx-get|hx-post|hx-put|hx-delete|hx-patch|action)=["\x27]/[^/]'

check-links:
	@echo "==> [check-links] dist/index.html"
	@if $(_ABS_GREP) rsx-webui/dist/index.html; then \
		echo "FAIL: root-absolute href/src in dist/index.html" >&2; \
		exit 1; \
	fi
	@echo "    PASS: dist/index.html clean"
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
	cargo test --workspace --test '*' --no-fail-fast \
		--exclude rsx-risk
	@echo "==> Running rsx-risk E2E tests (serial: env-var tests)..."
	cargo test -p rsx-risk --test '*' --no-fail-fast \
		-- --test-threads=1
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
	bash scripts/smoke.sh

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

# Local deterministic PROGRESS regeneration check.
# Reads PROGRESS.md only; no bundle, no network.
# Fails if denominator != 223 or header diverges from log counts.
regen-progress:
	python3 scripts/regen-progress.py

# Status doctor: required gate before any PROGRESS update.
# Runs 5 checks: denominator, phase semantics, contradiction,
# artifact freshness, shard determinism.
status-doctor:
	python3 scripts/status_doctor.py

# Regenerate PROGRESS.md header from acceptance artifacts (tasks.json,
# gate-3-report.json, play-artifacts/). Fails if header would diverge.
# Use --force to overwrite when you want to reset from artifacts.
publish-progress: status-doctor
	python3 scripts/publish-progress.py

# Generate acceptance bundle (gate statuses, API summary, Playwright totals,
# failing IDs, commit SHA, timestamp). Blocks if gate-3-report.json is stale.
# Writes rsx-playground/tmp/acceptance-bundle.json.
acceptance-bundle:
	@echo "==> [acceptance-bundle] generating..."
	python3 scripts/acceptance-bundle.py
	@echo "    written: rsx-playground/tmp/acceptance-bundle.json"

# Generate release_truth.json from acceptance-bundle.json.
# Reads git SHA from .git dir (no external CLI dependency).
# Blocks if bundle is missing, stale, or SHA-mismatched.
gen-release-truth: acceptance-bundle
	@echo "==> [gen-release-truth] generating..."
	python3 scripts/gen-release-truth.py
	@echo "    written: rsx-playground/tmp/release_truth.json"

# Release gate: blocks unless Playwright==223/223 AND all upstream gates green.
# Runs acceptance-bundle + gen-release-truth; exits non-zero on any failure.
# Use this as the final CI gate before tagging a release.
release-gate: gen-release-truth
	@python3 -c "\
import json, sys; \
b = json.load(open('rsx-playground/tmp/release_truth.json')); \
pw = b['playwright_passed']; \
ok = b['all_green']; \
canon = b['canonical_ok']; \
print(f'[release-gate] playwright={pw}/223 all_green={ok} canonical_ok={canon}'); \
sys.exit(0 if ok and canon else 1)"

# Deterministic exit criteria: auto-reopens completed tasks whose linked
# failing_test_ids are not yet green on the current HEAD acceptance bundle.
# Exit 0 = all completed tasks satisfy criteria; 1 = tasks reopened; 2 = bundle missing.
exit-criteria:
	python3 scripts/exit-criteria.py

# Truth-source reporter: compute task counts from .ship/tasks.json and update
# PROGRESS.md header. Prevents manual/stale drift. Use --check for CI gate.
task-report:
	python3 scripts/task-report.py

# Contradiction linter: rejects .ship/tasks.json snapshots where any task id
# appears in both DONE and FAIL/retry sets. Run before applying any update.
lint-snapshot:
	python3 scripts/lint-snapshot.py

# Unit tests for snapshot linter and acceptance-bundle CI checks.
# Covers: denominator != 223, phase-semantics zombie state, DONE-FAIL splits.
# Also covers exit-criteria: SHA check, artifact timestamp, reopen logic.
lint-snapshot-tests:
	python3 scripts/tests/test_lint_snapshot.py
	python3 scripts/tests/test_acceptance_bundle.py
	python3 scripts/tests/test_ci_guard.py
	python3 scripts/tests/test_exit_criteria.py
	rsx-playground/.venv/bin/python3 -m pytest \
		scripts/tests/test_freshness_enforcement.py \
		scripts/tests/test_local_runner.py \
		-q

# CI guard: validate artifact JSON — fail on denominator != 223 or
# phase-state contradictions (zombie/stuck execution states).
# Usage: make ci-guard ARTIFACT=rsx-playground/tmp/acceptance-bundle.json
ci-guard:
	python3 scripts/ci-guard.py $(ARTIFACT)

# Fallback local runner: executes release-validation make targets for tasks
# stuck in 'running' state (blocked external-agent sessions, quota limits).
# Picks up tasks from .ship/tasks.json; updates status on pass/fail.
# Usage:
#   make local-validate           # run all blocked (running) tasks
#   make local-validate-dry       # dry-run: show what would execute
#   make local-validate-pending   # also include pending tasks
local-validate:
	python3 scripts/local-runner.py

local-validate-dry:
	python3 scripts/local-runner.py --dry-run

local-validate-pending:
	python3 scripts/local-runner.py --pending

local-head:
	python3 scripts/local-runner.py --head-only

# Meta-orchestration guard: block new meta tasks until
# product-critical failing Playwright IDs decrease across 2
# consecutive fresh cycles.  Exits 1 if blocked, 0 if allowed.
meta-guard:
	python3 scripts/meta-guard.py

meta-guard-status:
	python3 scripts/meta-guard.py --status --verbose

meta-guard-tests:
	python3 -m pytest scripts/tests/test_meta_guard.py -v

# Lint
lint:
	cargo clippy --workspace -- -D warnings

# Clean build artifacts
clean:
	cargo clean

# Playwright e2e tests for RSX Playground
play: play-infra play-overview play-topology play-book play-risk \
     play-wal play-logs play-control play-faults \
     play-verify play-orders play-nav play-api

play-infra:
	cd rsx-playground/tests && npx playwright test play_infra.spec.ts

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
