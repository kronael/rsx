.PHONY: check test e2e integration wal smoke perf \
       lint clean play play-overview play-topology \
       play-book play-risk play-wal play-logs \
       play-control play-faults play-verify \
       play-orders play-nav play-api

# Type check only (fastest feedback, no codegen)
check:
	cargo check --workspace

# Unit tests (<5s)
test:
	cargo test --workspace

# E2E component tests (~30s)
e2e:
	cargo test --workspace --test '*' --no-fail-fast

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
