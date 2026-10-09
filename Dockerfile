# Build:  docker build -t bloogla .
# Run:    docker run -d --name bloogla -p 80:80 -p 443:443 -v bloogla:/app \
#           -e BLOOGLA_TLS_DOMAINS=example.com bloogla
# Then open the setup link from `docker logs bloogla` to finish setup.
# To try it without a domain: docker run -d -p 8080:8080 -v bloogla:/app bloogla
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release && mkdir -p /out/app

FROM gcr.io/distroless/cc-debian12:nonroot
# The image runs as an unprivileged user, so it must own its data folder.
COPY --from=build --chown=65532:65532 /out/app /app
WORKDIR /app
COPY --from=build /src/target/release/bloogla /usr/local/bin/bloogla
ENV BLOOGLA_HOST=0.0.0.0 \
    BLOOGLA_PORT=8080
EXPOSE 8080 80 443
VOLUME ["/app"]
ENTRYPOINT ["/usr/local/bin/bloogla"]
CMD ["serve"]
