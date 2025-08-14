CI := 1

.PHONY: check fmt clippy test audit deny security ci

fmt:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

audit:
	@command -v cargo-audit >/dev/null || (echo "Installing cargo-audit..." && cargo install cargo-audit)
	cargo audit

deny:
	@command -v cargo-deny >/dev/null || (echo "Installing cargo-deny..." && cargo install cargo-deny)
	cargo deny check

security: audit deny

check: fmt clippy test security

ci: check

# Development commands
dev-fmt:
	cargo fmt --all

dev-clippy:
	cargo clippy --workspace --all-targets --fix --allow-dirty

dev-test:
	cargo test --workspace

# Quick start helpers
quickstart: 
	mkdir -p data
	cargo run --bin hyperspot-server -- --config config/quickstart.yaml run

smoke-test:
	@echo "Starting smoke tests..."
	@./scripts/smoke-test.sh || echo "Note: Run 'make quickstart' in another terminal first"

