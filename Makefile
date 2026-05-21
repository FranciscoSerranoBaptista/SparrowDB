F = --features lmdb

.PHONY: build check test test-all sweep

build:
	cargo build --workspace $(F)
	cargo sweep -t 3

check:
	cargo check --workspace $(F)
	cargo sweep -t 3

test:
	cargo test --package sparrow-db $(F) --lib -- --test-threads=2
	cargo sweep -t 3

test-all:
	cargo test --workspace $(F) --lib -- --test-threads=2
	cargo sweep -t 3

sweep:
	cargo sweep -t 3
