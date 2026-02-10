.PHONY: test e2e integration wal smoke perf clean

# Unit tests (<5s) - fast feedback loop
test:
	cargo test --lib --bins

# E2E component tests (~30s) - complete order lifecycle with mocked components
e2e:
	cargo test --test '*' --no-fail-fast

# Integration tests (1-5min) - full system stack with testcontainers
integration:
	cargo test --test '*' -- --ignored --test-threads=1

# WAL correctness tests (<10s)
wal:
	cargo test -p rsx-dxs

# Smoke tests (<1min) - against deployed systems
smoke:
	@echo "Smoke tests not yet implemented"

# Performance benchmarks (long-running)
perf:
	cargo bench

# Clean build artifacts
clean:
	cargo clean
