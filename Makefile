.PHONY: install fmt fmt-check lint test benchmark build check tauri-dev

install:
	npm ci

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

lint: fmt-check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	npm run lint

test:
	cargo test --workspace --locked
	npm run test

benchmark:
	cargo run --locked -q -p loom-cli -- benchmark --corpus benchmarks/retrieval/v0/corpus --queries benchmarks/retrieval/v0/queries.jsonl

build:
	npm run build
	cargo check --workspace --locked

check: lint test benchmark build

tauri-dev:
	npm run tauri dev
