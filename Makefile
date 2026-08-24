.PHONY: install fmt fmt-check lint test benchmark build roadmap-check check verify-device tauri-dev

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

roadmap-check:
	python3 scripts/roadmap.py --validate-only
	python3 -m unittest discover -s tests -v

verify-device:
	bash scripts/verify-device.sh

check: lint test benchmark build roadmap-check

tauri-dev:
	npm run tauri dev
