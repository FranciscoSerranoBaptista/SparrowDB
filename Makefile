F = --features lmdb

.PHONY: build check test test-all sweep docker-build docker-up docker-down docker-logs

build:
	cargo build --workspace $(F)
	cargo sweep -t 3

check:
	cargo check --workspace $(F)
	cargo sweep -t 3

test:
	cargo test --package sparrow-core $(F) -- --test-threads=2
	cargo sweep -t 3

test-all:
	cargo test --workspace $(F) -- --test-threads=2
	cargo sweep -t 3

sweep:
	cargo sweep -t 3

docker-build:
	docker compose build

docker-up:
	docker compose up -d

docker-down:
	docker compose down

docker-logs:
	docker compose logs -f
