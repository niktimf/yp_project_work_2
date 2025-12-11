FROM rust:1.90-slim as builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY server/Cargo.toml server/
COPY client/Cargo.toml client/
COPY common/Cargo.toml common/

RUN mkdir -p server/src client/src common/src && \
    echo "fn main() {}" > server/src/main.rs && \
    echo "fn main() {}" > client/src/main.rs && \
    echo "" > common/src/lib.rs && \
    cargo build --release -p server && \
    rm -rf server/src client/src common/src

COPY server/src server/src
COPY common/src common/src

RUN touch server/src/main.rs && \
    cargo build --release -p server

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/server /usr/local/bin/quote-server

EXPOSE 5000/tcp
EXPOSE 5001/udp

ENV RUST_LOG=info

CMD ["quote-server"]
