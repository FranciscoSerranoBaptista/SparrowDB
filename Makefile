F = --features lmdb

.PHONY: build check test test-all sweep \
        sdk-build sdk-check \
        docker-build docker-up docker-down docker-logs \
        bench bench-update bench-diff

build:
	cargo build --workspace $(F)
	pnpm build
	cargo sweep -t 3

check:
	cargo check --workspace $(F)
	pnpm type-check
	cargo sweep -t 3

test:
	cargo test --package sparrow-core $(F) -- --test-threads=2
	cargo sweep -t 3

test-all:
	cargo test --workspace $(F) -- --test-threads=2
	cargo sweep -t 3

sweep:
	cargo sweep -t 3

sdk-build:
	pnpm build

sdk-check:
	pnpm type-check

docker-build:
	docker compose build

docker-up:
	docker compose up -d

docker-down:
	docker compose down

docker-logs:
	docker compose logs -f

## Run CPU benchmarks (no comparison — just measure)
bench:
	cargo bench -p sparrow-benches --bench compiler --bench traversal --bench write_pipeline --features cpu

## Re-run benchmarks, save new baselines, stage for commit.
## Review the diff with `git diff --staged`, then commit manually:
##   git commit -m "perf(benches): update baselines"
bench-update:
	cargo bench -p sparrow-benches --bench compiler --bench traversal --bench write_pipeline --features cpu -- --save-baseline main
	@find target/criterion -type f -name "estimates.json" -path "*/main/*" | while read f; do \
	  rel="$${f#target/criterion/}"; \
	  dir="crates/sparrow-benches/baselines/$$(dirname $$rel)"; \
	  mkdir -p "$$dir"; \
	  cp "$$f" "$$dir/"; \
	  cp "$$(dirname $$f)/benchmark.json" "$$dir/" 2>/dev/null || true; \
	  cp "$$(dirname $$f)/sample.json" "$$dir/" 2>/dev/null || true; \
	  cp "$$(dirname $$f)/tukey.json" "$$dir/" 2>/dev/null || true; \
	done
	git add crates/sparrow-benches/baselines/
	@echo "Baselines staged. Run: git commit -m 'perf(benches): update baselines'"

## Diff the current run against committed main baselines (same as CI does).
## Requires critcmp: cargo install critcmp
bench-diff:
	@find crates/sparrow-benches/baselines -type f -name "*.json" | while read f; do \
	  rel="$${f#crates/sparrow-benches/baselines/}"; \
	  dir="target/criterion/$$(dirname $$rel)"; \
	  mkdir -p "$$dir"; \
	  cp "$$f" "$$dir/"; \
	done
	cargo bench -p sparrow-benches --bench compiler --bench traversal --bench write_pipeline --features cpu -- --save-baseline current
	critcmp main current
