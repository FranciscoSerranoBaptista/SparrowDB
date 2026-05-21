FROM lukemathwalker/cargo-chef:latest-rust-1.88 AS chef
WORKDIR /build

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# --- dependency cache layer ---
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json --bin sparrow-container

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --package sparrow-container --recipe-path recipe.json

COPY . .
RUN cargo build --release --package sparrow-container

# --- runtime image ---
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/sparrow-container /usr/local/bin/sparrow-container

RUN mkdir -p /data

ENV SPARROW_DATA_DIR=/data
ENV SPARROW_PORT=6969

EXPOSE 6969

CMD ["sparrow-container"]
