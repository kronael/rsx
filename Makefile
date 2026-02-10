.PHONY: check test e2e integration wal smoke perf \
       lint clean

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
