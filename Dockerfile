FROM rust:1.75-slim AS builder

WORKDIR /usr/src/bloogla

RUN apt-get update && apt-get install -y pkg-config libssl-dev libsqlite3-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

COPY . .

RUN touch src/main.rs
RUN cargo build --release

FROM debian:bookworm-slim

WORKDIR /app

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libsqlite3-0 \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /app/uploads /app/data

COPY --from=builder /usr/src/bloogla/target/release/bloogla /app/bloogla
COPY --from=builder /usr/src/bloogla/static /app/static

RUN useradd -m -u 10001 blooglauser && \
    chown -R blooglauser:blooglauser /app
USER blooglauser

EXPOSE 8080

CMD ["./bloogla"]
