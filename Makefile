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
	cargo bench -p sparrow-benches --features cpu

## Re-run benchmarks, save new baselines, stage for commit.
## Review the diff with `git diff --staged`, then commit manually:
##   git commit -m "perf(benches): update baselines"
bench-update:
	cargo bench -p sparrow-benches --features cpu -- --save-baseline main
	@for bench in compiler traversal write_pipeline; do \
	  mkdir -p crates/sparrow-benches/baselines/$$bench; \
	  if [ -d target/criterion/$$bench/main ]; then \
	    cp -r target/criterion/$$bench/main crates/sparrow-benches/baselines/$$bench/; \
	  fi; \
	done
	git add crates/sparrow-benches/baselines/
	@echo "Baselines staged. Run: git commit -m 'perf(benches): update baselines'"

## Diff the current run against committed main baselines (same as CI does).
## Requires critcmp: cargo install critcmp
bench-diff:
	rsync -av crates/sparrow-benches/baselines/ target/criterion/
	cargo bench -p sparrow-benches --features cpu -- --save-baseline current
	critcmp main current
