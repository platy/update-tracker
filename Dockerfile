FROM lukemathwalker/cargo-chef:latest-rust-1.58.1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . /app
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json -p update-tracker
# Build application
COPY . /app
RUN cargo build --release -p update-tracker

# We do not need the Rust toolchain to run the binary!
FROM debian:bullseye-slim AS runtime
RUN apt-get update && apt-get install -y openssl git && rm -rf /var/lib/apt/lists/*
ENV LISTEN_ADDR 0.0.0.0:80
WORKDIR /app/server
ENTRYPOINT ["/usr/local/bin/update-tracker"]
COPY ./server/static /app/server/static
COPY --from=builder /app/target/release/update-tracker /usr/local/bin
