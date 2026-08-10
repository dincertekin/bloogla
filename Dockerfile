# --- STAGE 1: Build Stage ---
FROM rust:1.75-slim as builder

WORKDIR /usr/src/bloogla

# Bağımlılık derlemelerini önbelleğe almak için geçici dummy yapısı
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

# Gerçek kaynak kodları kopyala ve yayın (release) ikili dosyasını derle
COPY . .
RUN touch src/main.rs
RUN cargo build --release

# --- STAGE 2: Runtime Stage ---
FROM debian:bookworm-slim

WORKDIR /app

# Gerekli runtime kütüphaneleri (SQLite & SSL desteği için)
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

# Builder aşamasından sadece derlenmiş binary'yi al
COPY --from=builder /usr/src/bloogla/target/release/bloogla /app/bloogla
COPY --from=builder /usr/src/bloogla/static /app/static
COPY --from=builder /usr/src/bloogla/templates /app/templates

EXPOSE 8080

CMD ["./bloogla"]
