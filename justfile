# Megabase Justfile
# Run commands with: just <command>

# Default: show available commands
default:
    @just --list

# Build the project
build:
    cargo build

# Build release binary
release:
    cargo build --release

# Run the binary (dev mode)
run:
    cargo run

# Run tests
test:
    cargo test --all

# Run clippy lints
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Format code
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Full CI check (format, lint, test)
ci: fmt-check lint test

# Extract units from vendor sources
extract-units:
    python3 scripts/extract-units.py

# Generate coverage treemaps
treemap:
    python3 scripts/generate-treemap.py

# Update coverage (extract + treemap)
coverage: extract-units treemap

# Start the judge reference stack
judge-up:
    cd judge && docker compose up -d

# Stop the judge reference stack
judge-down:
    cd judge && docker compose down

# Run judge comparison tests
judge: judge-up
    @echo "Waiting for services..."
    @sleep 10
    python3 judge/compare.py --verbose

# Run judge with JSON output
judge-json:
    python3 judge/compare.py --json

# Clean build artifacts
clean:
    cargo clean
    rm -rf coverage/*.svg coverage/summary.json

# Show current coverage summary
status:
    @cat coverage/summary.json | python3 -m json.tool
