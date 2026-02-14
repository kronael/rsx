# RSX Documentation

This directory contains the MkDocs-based documentation for RSX Exchange.

## Quick Start

### Install and Serve

```bash
# From project root
./scripts/serve-docs.sh
```

This will:
1. Create a Python virtual environment (`.mkdocs-venv/`)
2. Install MkDocs Material theme
3. Start the documentation server at http://localhost:8001

### Manual Installation

If the script fails, install MkDocs manually:

```bash
# Install python3-venv (Debian/Ubuntu)
sudo apt install python3-venv

# Create virtual environment
python3 -m venv .mkdocs-venv

# Activate and install MkDocs
source .mkdocs-venv/bin/activate
pip install mkdocs-material

# Serve documentation
mkdocs serve --dev-addr 0.0.0.0:8001
```

### System-wide Installation

Alternatively, install MkDocs globally:

```bash
pip install --user mkdocs-material
mkdocs serve --dev-addr 0.0.0.0:8001
```

## Documentation Structure

```
docs/
├── index.md                    Landing page
├── getting-started/
│   ├── README.md              Overview
│   ├── architecture.md        System architecture
│   └── quickstart.md          Quick start guide
├── specs/
│   ├── v1/                    Version 1 specs
│   │   ├── README.md          Spec overview
│   │   ├── ARCHITECTURE.md    Core architecture
│   │   ├── ORDERBOOK.md       Orderbook spec
│   │   ├── RISK.md            Risk engine spec
│   │   ├── DXS.md             WAL spec
│   │   ├── CMP.md             CMP protocol spec
│   │   └── ...                (40+ spec files)
│   └── v2/                    Version 2 specs (future)
├── blog/
│   ├── README.md              Blog overview
│   ├── 01-design-philosophy.md
│   ├── 02-matching-engine.md
│   └── ...                    (25+ blog posts)
├── guides/
│   ├── operations.md          Operations runbook
│   ├── monitoring.md          Monitoring guide
│   └── deployment.md          Deployment guide
├── crates/
│   ├── README.md              Crate overview
│   ├── rsx-matching.md        Matching engine crate
│   └── rsx-cli.md             CLI tool crate
└── references/
    ├── GUARANTEES.md          System guarantees
    ├── CRASH-SCENARIOS.md     Crash scenarios
    └── PROGRESS.md            Implementation progress
```

## Building Documentation

### Development Server

```bash
mkdocs serve --dev-addr 0.0.0.0:8001
```

Access at http://localhost:8001. Live reload enabled.

### Build Static Site

```bash
mkdocs build
```

Output: `docs/site/` (gitignored)

### Validate Configuration

```bash
mkdocs build --strict
```

This will fail if there are broken links or missing files.

## Navigation Structure

Documentation is organized into tabs:

1. **Home** - Landing page with quick links
2. **Getting Started** - Architecture, overview, quick start
3. **Specifications** - Complete technical specs by component
4. **Blog** - Design philosophy, technical posts
5. **Guides** - Operations, monitoring, deployment
6. **Crate Documentation** - Per-crate architecture
7. **References** - Guarantees, crash scenarios, progress

## Playground Integration

The RSX Playground dashboard includes a "Docs" link that opens the documentation in a new tab at http://localhost:8001.

To use:

1. Start the playground: `cd rsx-playground && uv run server.py`
2. Start the docs server: `./scripts/serve-docs.sh`
3. Open http://localhost:3000 (playground)
4. Click "Docs" tab to open documentation

## Theme

MkDocs Material theme with:

- Dark/light mode toggle
- Navigation tabs
- Search
- Syntax highlighting
- Code copy buttons
- Responsive design

## Markdown Extensions

Enabled extensions:

- **Code blocks** - Syntax highlighting, line numbers, copy button
- **Admonitions** - Note, warning, danger callouts
- **Tables** - GitHub-flavored markdown tables
- **Task lists** - `- [ ]` checkbox syntax
- **Mermaid** - Diagram support (future)
- **TOC** - Auto-generated table of contents

## Source Files

Documentation is copied/symlinked from:

- Root `README.md` → `getting-started/README.md`
- Root `ARCHITECTURE.md` → `getting-started/architecture.md`
- `specs/v1/*` → `specs/v1/*`
- `blog/*` → `blog/*`
- `RECOVERY-RUNBOOK.md` → `guides/operations.md`
- `GUARANTEES.md` → `references/GUARANTEES.md`
- Crate `ARCHITECTURE.md` files → `crates/`

Original files are NOT deleted. Docs are copies.

## Updating Documentation

### Add New Spec

1. Create spec file in `specs/v1/NEW-SPEC.md`
2. Copy to `docs/specs/v1/NEW-SPEC.md`
3. Add to `mkdocs.yml` navigation:
   ```yaml
   - New Spec: specs/v1/NEW-SPEC.md
   ```
4. Rebuild: `mkdocs serve`

### Add New Blog Post

1. Create post in `blog/19-new-post.md`
2. Copy to `docs/blog/19-new-post.md`
3. Add to `mkdocs.yml` navigation
4. Update `blog/README.md` index

### Update Existing Doc

1. Edit source file (e.g., `specs/v1/ORDERBOOK.md`)
2. Copy to docs: `cp specs/v1/ORDERBOOK.md docs/specs/v1/`
3. Live reload will update browser

## Troubleshooting

### Port 8001 Already in Use

```bash
mkdocs serve --dev-addr 0.0.0.0:8002
```

Or kill the existing process:

```bash
lsof -ti:8001 | xargs kill
```

### Missing Python Packages

```bash
source .mkdocs-venv/bin/activate
pip install mkdocs-material
```

### Broken Links

Run strict build to find broken links:

```bash
mkdocs build --strict
```

### Permission Errors

If virtual environment creation fails:

```bash
sudo apt install python3-venv
```

## CI/CD Integration

To build docs in CI:

```yaml
- name: Build docs
  run: |
    pip install mkdocs-material
    mkdocs build --strict
```

To deploy to GitHub Pages:

```yaml
- name: Deploy docs
  run: mkdocs gh-deploy --force
```

## License

Documentation is MIT licensed (same as RSX codebase).
