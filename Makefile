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
	@echo "Unit Tests (fast):"
	@echo "  make test          - All unit tests (Rust + Python API, ~10s)"
	@echo "  make check         - Type check only (fastest, no tests)"
	@echo "  make api-unit      - API unit tests only (~5s, 230 tests)"
	@echo ""
	@echo "E2E Tests (comprehensive):"
	@echo "  make e2e           - All E2E (Rust + API + Playwright, ~2min)"
	@echo "  make play          - Playwright E2E only (149 tests, ~30s)"
	@echo "  make api-integration - API integration tests only"
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

# Unit tests - ALL unit tests (Rust + Python API unit tests)
test:
	@echo "==> Running Rust unit tests..."
	cargo test --workspace --lib
	@echo ""
	@echo "==> Running Python API unit tests..."
	cd rsx-playground && uv run pytest tests/api_processes_test.py tests/api_risk_test.py tests/api_wal_test.py tests/api_logs_metrics_test.py tests/api_verify_test.py -v --tb=short -x

# E2E tests - ALL E2E tests (Rust + Python API integration + Playwright)
e2e:
	@echo "==> Running Rust E2E tests..."
	cargo test --workspace --test '*' --no-fail-fast
	@echo ""
	@echo "==> Running Python API integration tests..."
	cd rsx-playground && uv run pytest tests/api_orders_test.py tests/api_integration_test.py tests/api_edge_cases_test.py -v --tb=short -x
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

# API unit tests only (fast, part of 'make test')
api-unit:
	@echo "==> Running API unit tests..."
	cd rsx-playground && uv run pytest tests/api_processes_test.py tests/api_risk_test.py tests/api_wal_test.py tests/api_logs_metrics_test.py tests/api_verify_test.py -v --tb=short

# API integration tests (part of 'make e2e')
api-integration:
	@echo "==> Running API integration tests..."
	cd rsx-playground && uv run pytest tests/api_orders_test.py tests/api_integration_test.py tests/api_edge_cases_test.py -v --tb=short

# Stress tests with latency measurement (3+ minutes)
api-stress:
	@echo "==> Running stress tests (may take 3+ minutes)..."
	cd rsx-playground && uv run pytest tests/api_orders_test.py -k stress -v -s
