# The published Docker image (ghcr.io/dincertekin/bloogla), made by
# .github/workflows/release.yml from the signed Linux programs of the
# release, for Intel/AMD and ARM. The programs are fully static, so the
# smallest base image is enough. (To build from source, use ../Dockerfile.)
#
# Run: docker run -d --name bloogla -p 80:80 -p 443:443 -v bloogla:/app \
#        -e BLOOGLA_TLS_DOMAINS=example.com ghcr.io/dincertekin/bloogla
FROM gcr.io/distroless/static-debian12:nonroot
ARG TARGETARCH
# The image runs as an unprivileged user, so it must own its data folder.
COPY --chown=65532:65532 app /app
WORKDIR /app
COPY --chmod=755 ${TARGETARCH}/bloogla /usr/local/bin/bloogla
ENV BLOOGLA_HOST=0.0.0.0 \
    BLOOGLA_PORT=8080
EXPOSE 8080 80 443
VOLUME ["/app"]
ENTRYPOINT ["/usr/local/bin/bloogla"]
CMD ["serve"]
