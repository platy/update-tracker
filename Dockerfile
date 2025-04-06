FROM lukemathwalker/cargo-chef:latest-rust-1.58.1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . /app
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json -p govdiff_server
# Build application
COPY . /app
RUN cargo build --release -p govdiff_server

# We do not need the Rust toolchain to run the binary!
FROM debian:bullseye-slim AS runtime
RUN apt-get update && apt-get install -y openssl git && rm -rf /var/lib/apt/lists/*
ENV LISTEN_ADDR 0.0.0.0:80
WORKDIR /app/govdiff_server
ENTRYPOINT ["/usr/local/bin/update-tracker"]
COPY ./govdiff_server/static /app/govdiff_server/static
COPY --from=builder /app/target/release/update-tracker /usr/local/bin
