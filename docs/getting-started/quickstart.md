# Quick Start

This guide will help you build and run RSX locally.

## Prerequisites

- Rust 1.70+ (2021 edition)
- Cargo
- PostgreSQL 15+
- Python 3.10+ (for playground dashboard)

## Build

```bash
# Clone the repository
git clone https://github.com/yourusername/rsx.git
cd rsx

# Check all crates compile
cargo check --workspace

# Run tests
cargo test --workspace

# Build all binaries (debug mode, faster compilation)
cargo build --workspace
```

## Database Setup

```bash
# Create database
createdb rsx

# Set DATABASE_URL
export DATABASE_URL="postgres://rsx:rsx@127.0.0.1:5432/rsx"

# Run migrations (if applicable)
# psql $DATABASE_URL < migrations/schema.sql
```

## Run Components

Each component runs as a separate process. Start them in order:

```bash
# Terminal 1: Risk Engine
cd rsx-risk
cargo run -- config.toml

# Terminal 2: Matching Engine (per symbol)
cd rsx-matching
cargo run -- config.toml

# Terminal 3: Gateway
cd rsx-gateway
cargo run -- config.toml

# Terminal 4: Market Data
cd rsx-marketdata
cargo run -- config.toml

# Terminal 5: Mark Price
cd rsx-mark
cargo run -- config.toml
```

## Playground Dashboard

The playground provides a web UI for testing and monitoring:

```bash
cd rsx-playground
uv venv
source .venv/bin/activate
uv pip install -r requirements.txt
uv run server.py
```

Open http://localhost:3000 in your browser.

## Configuration

Each component reads TOML config from first CLI argument:

```toml
# config.toml example
[risk]
shard_id = 0
listen_addr = "127.0.0.1:9000"
wal_dir = "./tmp/wal"

[gateway]
ws_addr = "0.0.0.0:8080"
risk_addr = "127.0.0.1:9000"
jwt_secret = "dev-secret-key"
```

See component specs for full config options.

## Testing

```bash
# Unit tests (fast)
cargo test -p rsx-book
cargo test -p rsx-matching
cargo test -p rsx-dxs

# Integration tests
cargo test --workspace --test '*_test'

# E2E tests (requires running system)
cd rsx-playground
pytest tests/
```

## Development Workflow

```bash
# 1. Check compiles (fastest feedback)
cargo check

# 2. Run tests
cargo test -p <crate-name>

# 3. Format
cargo fmt --all

# 4. Lint
cargo clippy --all-targets -- -D warnings

# 5. Build
cargo build --workspace
```

## Next Steps

- Read [Architecture](architecture.md) to understand system design
- Check [Specifications](../specs/v1/README.md) for component details
- Explore [Blog Posts](../blog/README.md) for design decisions
- Review [Operations Guide](../guides/operations.md) for production deployment
