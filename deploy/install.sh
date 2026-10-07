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
#
# Options (environment variables):
#   HTTPS=builtin   Let Bloogla handle HTTPS itself instead of installing Caddy
#   EMAIL=you@...   Contact address for Let's Encrypt (built-in HTTPS)
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
    ARCHIVE="bloogla-$TARGET.tar.gz"
    RELEASE="https://github.com/$REPO/releases/latest/download"
    curl -fsSL -o "$TMP/$ARCHIVE" "$RELEASE/$ARCHIVE"
    curl -fsSL -o "$TMP/$ARCHIVE.sha256" "$RELEASE/$ARCHIVE.sha256"
    # Refuse a download that is damaged or doesn't match the published checksum.
    (cd "$TMP" && sha256sum -c "$ARCHIVE.sha256" >/dev/null) || {
        echo "The download doesn't match its checksum. Nothing was installed; please try again."
        exit 1
    }
    tar -xzf "$TMP/$ARCHIVE" -C "$TMP"
    BINARY="$TMP/bloogla"
fi

echo "==> Installing binary"
install -m 0755 "$BINARY" /usr/local/bin/bloogla

echo "==> Creating bloogla user and $DATA_DIR"
id bloogla >/dev/null 2>&1 || useradd --system --home "$DATA_DIR" --shell /usr/sbin/nologin bloogla
mkdir -p "$DATA_DIR"
chown bloogla:bloogla "$DATA_DIR"

# HTTPS: Caddy by default (shares the server nicely with other sites); Bloogla's
# built-in HTTPS when asked for (HTTPS=builtin) or when Caddy isn't available.
HTTPS="${HTTPS:-caddy}"
if [ "$HTTPS" = caddy ] && ! command -v caddy >/dev/null 2>&1; then
    echo "==> Installing Caddy for HTTPS"
    apt-get update -q
    if ! apt-get install -y -q caddy; then
        echo "    Caddy isn't available from apt here; using Bloogla's built-in HTTPS instead."
        HTTPS=builtin
    fi
fi

if [ "$HTTPS" = builtin ]; then
    SERVICE_ENV="Environment=BLOOGLA_TLS_DOMAINS=$DOMAIN
Environment=BLOOGLA_TLS_EMAIL=${EMAIL:-}
# Allow binding ports 80 and 443 without running as root.
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE"
else
    SERVICE_ENV="Environment=BLOOGLA_HOST=127.0.0.1
Environment=BLOOGLA_PORT=8080
Environment=BLOOGLA_BASE_URL=https://$DOMAIN
Environment=BLOOGLA_PRODUCTION=true"
fi

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
$SERVICE_ENV
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

if [ "$HTTPS" = caddy ]; then
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
fi

STARTED_AT=$(date '+%Y-%m-%d %H:%M:%S')
systemctl daemon-reload
systemctl enable bloogla >/dev/null 2>&1
systemctl restart bloogla
if [ "$HTTPS" = caddy ]; then
    systemctl reload caddy || systemctl restart caddy
fi

# A new site prints a one-time setup link (with a code, so nobody else can
# claim the site first). Wait a few seconds for it to show up in the log.
SETUP_LINK=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
    SETUP_LINK=$(journalctl -u bloogla --since "$STARTED_AT" --no-pager -o cat 2>/dev/null \
        | grep -o 'https\?://[^ ]*/setup?code=[0-9a-f]*' | tail -n 1)
    [ -n "$SETUP_LINK" ] && break
    sleep 1
done

echo
if [ -n "$SETUP_LINK" ]; then
    echo "Bloogla is running. Open this link to create your account (it works once):"
    echo
    echo "    $SETUP_LINK"
    echo
else
    echo "Bloogla is running at https://$DOMAIN"
fi
echo "Make sure $DOMAIN points (DNS A/AAAA record) to this server so HTTPS can be issued."
echo "Logs: journalctl -u bloogla -f    Backups: $DATA_DIR/data/backups"
