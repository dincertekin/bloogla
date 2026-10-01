#!/bin/sh
# Install Bloogla on a Debian/Ubuntu VPS with HTTPS via Caddy.
#
# On the server, as root:
#   curl -fsSL https://raw.githubusercontent.com/dincertekin/bloogla/main/deploy/install.sh | sh -s example.com
#
# Or with a binary you built yourself, placed next to this script:
#   scp target/release/bloogla deploy/install.sh root@your-server:
#   ssh root@your-server 'sh install.sh example.com'
#
# Safe to re-run: it upgrades the binary and keeps your data.
set -eu

DOMAIN="${1:?Usage: sh install.sh your-domain.com}"
BINARY="${BINARY:-./bloogla}"
REPO="${BLOOGLA_REPO:-dincertekin/bloogla}"
DATA_DIR=/var/lib/bloogla

[ "$(id -u)" -eq 0 ] || { echo "Run as root (or with sudo)."; exit 1; }

if [ ! -x "$BINARY" ]; then
    case "$(uname -m)" in
        x86_64 | amd64) TARGET=x86_64-unknown-linux-musl ;;
        aarch64 | arm64) TARGET=aarch64-unknown-linux-musl ;;
        *) echo "No prebuilt binary for $(uname -m). Build one with: cargo build --release"; exit 1 ;;
    esac
    command -v curl >/dev/null 2>&1 || { apt-get update -q && apt-get install -y -q curl; }
    echo "==> Downloading the latest Bloogla for $TARGET"
    TMP=$(mktemp -d)
    curl -fsSL "https://github.com/$REPO/releases/latest/download/bloogla-$TARGET.tar.gz" | tar -xz -C "$TMP"
    BINARY="$TMP/bloogla"
fi

echo "==> Installing binary"
install -m 0755 "$BINARY" /usr/local/bin/bloogla

echo "==> Creating bloogla user and $DATA_DIR"
id bloogla >/dev/null 2>&1 || useradd --system --home "$DATA_DIR" --shell /usr/sbin/nologin bloogla
mkdir -p "$DATA_DIR"
chown bloogla:bloogla "$DATA_DIR"

echo "==> Installing systemd service"
cat > /etc/systemd/system/bloogla.service <<UNIT
[Unit]
Description=Bloogla blog
After=network-online.target
Wants=network-online.target

[Service]
User=bloogla
Group=bloogla
WorkingDirectory=$DATA_DIR
ExecStart=/usr/local/bin/bloogla serve
Environment=BLOOGLA_HOST=127.0.0.1
Environment=BLOOGLA_PORT=8080
Environment=BLOOGLA_BASE_URL=https://$DOMAIN
Environment=BLOOGLA_PRODUCTION=true
Restart=on-failure
RestartSec=2
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ReadWritePaths=$DATA_DIR

[Install]
WantedBy=multi-user.target
UNIT

echo "==> Installing Caddy for HTTPS"
if ! command -v caddy >/dev/null 2>&1; then
    apt-get update -q
    apt-get install -y -q caddy || {
        echo "Could not install Caddy from apt. See https://caddyserver.com/docs/install"
        exit 1
    }
fi

CADDYFILE=/etc/caddy/Caddyfile
SITE_BLOCK="$DOMAIN {
    encode zstd gzip
    reverse_proxy 127.0.0.1:8080
}"
if [ ! -f "$CADDYFILE" ] || grep -q "The Caddyfile is an easy way" "$CADDYFILE"; then
    # Stock Caddyfile from the package: replace it (keeping a copy).
    [ -f "$CADDYFILE" ] && cp "$CADDYFILE" "$CADDYFILE.orig"
    printf '%s\n' "$SITE_BLOCK" > "$CADDYFILE"
elif grep -q "^$DOMAIN" "$CADDYFILE"; then
    echo "    $CADDYFILE already has a block for $DOMAIN; leaving it unchanged."
else
    # Existing sites are kept; ours is added at the end.
    printf '\n%s\n' "$SITE_BLOCK" >> "$CADDYFILE"
fi

systemctl daemon-reload
systemctl enable bloogla >/dev/null 2>&1
systemctl restart bloogla
systemctl reload caddy || systemctl restart caddy

echo
echo "Bloogla is running. Open https://$DOMAIN in your browser to finish setup."
echo "Make sure $DOMAIN points (DNS A/AAAA record) to this server so HTTPS can be issued."
echo "Logs: journalctl -u bloogla -f    Backups: $DATA_DIR/data/backups"
