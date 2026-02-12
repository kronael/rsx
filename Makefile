.PHONY: check test e2e integration wal smoke perf \
       lint clean play play-overview play-topology \
       play-book play-risk play-wal play-logs \
       play-control play-faults play-verify \
       play-orders play-nav play-api \
       api-unit api-integration api-stress help

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
	@echo "Quality:"
	@echo "  make lint          - Run clippy with warnings as errors"
	@echo "  make perf          - Run performance benchmarks"
	@echo "  make clean         - Clean build artifacts"
	@echo ""
	@echo "Individual Playwright Tests:"
	@echo "  make play-orders, play-control, play-overview, play-book,"
	@echo "  play-risk, play-wal, play-logs, play-verify, play-topology,"
	@echo "  play-faults, play-nav, play-api"

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

# Performance benchmarks
perf:
	cargo bench

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
