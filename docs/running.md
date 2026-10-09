# Running Bloogla

Everything about putting Bloogla on a server and keeping it running. New
here? The [README](../README.md#quick-start) has the short version.

- [On a server](#on-a-server)
- [Configuration](#configuration)
- [Commands](#commands)
- [Updating](#updating)
- [Backups](#backups)

## On a server

There are two ways: run the single binary, or use Docker. Either way, point your domain's DNS at the server first.

**The binary.** Download `bloogla-x86_64-linux` (or `bloogla-aarch64-linux` for ARM servers) from the [latest release](https://github.com/dincertekin/bloogla/releases/latest), rename it to `bloogla`, make it runnable (`chmod +x bloogla`) and start it in an empty folder on the server. (Or build it yourself with `cargo build --release`.) Bloogla keeps everything in that folder (`data/`, `uploads/`, `themes/`). With `BLOOGLA_TLS_DOMAINS` it gets its own HTTPS certificate from Let's Encrypt, so nothing else is needed:

```bash
BLOOGLA_TLS_DOMAINS=example.com ./bloogla
# then open the setup link it prints (https://example.com/setup?code=...)
```

Ports 80 and 443 must be open. Keep it running with your server's usual tool (for example a systemd service or `tmux`).

**Docker** (no building needed):

```bash
docker run -d --name bloogla --restart unless-stopped -p 80:80 -p 443:443 \
  -v bloogla:/app -e BLOOGLA_TLS_DOMAINS=example.com ghcr.io/dincertekin/bloogla
# then open the setup link from `docker logs bloogla` to create your account
```

To build the image yourself instead: `docker build -t bloogla .`. Without a domain (to try it locally), leave out `BLOOGLA_TLS_DOMAINS` and use `-p 8080:8080`.

## Configuration

Set via environment variables:

| Variable                 | Default                  | Description                                                   |
| ------------------------ | ------------------------ | ------------------------------------------------------------- |
| `BLOOGLA_HOST`           | `0.0.0.0`                | Bind address. Use `127.0.0.1` behind a reverse proxy.         |
| `BLOOGLA_PORT`           | `8080`                   | Listen port.                                                  |
| `BLOOGLA_BASE_URL`       | `http://localhost:$PORT` | Public URL, used in links, feeds, SEO tags and CSRF checks.   |
| `BLOOGLA_PRODUCTION`     | `false`                  | `true` makes session cookies HTTPS-only and enables HSTS.     |
| `BLOOGLA_AUTO_BACKUP`    | `true`                   | Daily database backup to `data/backups/` (keeps 7).           |
| `BLOOGLA_ADMIN_EMAIL`    |                          | With `BLOOGLA_ADMIN_PASSWORD`, creates the admin on first start instead of the browser setup page. |
| `BLOOGLA_ADMIN_PASSWORD` |                          | See above.                                                    |
| `BLOOGLA_ADMIN_NAME`     |                          | Display name for scripted setup.                              |
| `BLOOGLA_BLOG_NAME`      | `My Blog`                | Site title for scripted setup.                                |
| `BLOOGLA_TLS_DOMAINS`    |                          | Comma-separated domains. Turns on built-in HTTPS with automatic Let's Encrypt certificates (port 443, and 80 redirects). |
| `BLOOGLA_TLS_EMAIL`      |                          | Contact address for certificate expiry notices.               |
| `BLOOGLA_TLS_STAGING`    | `false`                  | Use Let's Encrypt's test server while trying things out.      |
| `BLOOGLA_HTTP_PORT`      | `80`                     | Port that redirects to HTTPS when built-in HTTPS is on.       |
| `BLOOGLA_LOG`            | `info`                   | Log level filter, e.g. `debug` or `warn`.                     |
| `BLOOGLA_LOG_FORMAT`     | text                     | `json` for one JSON object per line.                          |

Bloogla keeps its files in the folder it is started from: `data/` (database and backups), `uploads/` and `themes/`.

## Commands

```bash
bloogla                          # start the server (same as `bloogla serve`)
bloogla backup [FILE]            # consistent copy of the live database
bloogla reset-password [EMAIL]   # set a new admin password and sign out all sessions
bloogla disable-2fa EMAIL        # turn off two-factor login for someone locked out
```

## Updating

**Settings → Updates** shows your version and has **Check for updates**, which asks GitHub for the newest release. New versions also show on the Dashboard, with a badge next to it in the menu. Two checkboxes there, both off at first: **Check for new versions every day**, and **Install new versions automatically**. Bloogla never checks on its own unless the first one is ticked. GitHub only learns your server's address and Bloogla's version.

How a new version gets installed depends on how Bloogla runs (download a backup first, in **Settings → Backup**):

- **The binary on Linux:** click **Install Bloogla 1.2.0** (or let the automatic install do it). Bloogla downloads the new program, checks that it's signed with Bloogla's release key (anything else is refused), swaps it in and restarts in place; the site is down for a few seconds. The previous program stays next to it as `bloogla.old`: to go back, stop Bloogla and rename it to `bloogla`. Bloogla must be allowed to write to the folder its program is in.
- **Docker:** a container can't replace itself, so: `docker pull ghcr.io/dincertekin/bloogla`, remove the container (`docker rm -f bloogla`) and run the same `docker run` command again. Your site lives in the `bloogla` volume, so nothing is lost.
- **Anything else** (macOS, a folder Bloogla can't write to): stop Bloogla, replace the program with the new one and start it again in the same folder.

The database is upgraded automatically on start.

## Backups

- **Download everything:** admins get the database and all images as one
  `.zip` in **Settings → Backup**. To restore it, stop Bloogla, unpack the
  `.zip` into the folder Bloogla runs in, and start it again.
- **On the server:** Bloogla also keeps a daily copy of the database in
  `data/backups/` (the last 7; turn it off with `BLOOGLA_AUTO_BACKUP=false`),
  and `bloogla backup [FILE]` makes one any time.
- **Keep `data/secret.key` with your backups.** The mail server password and
  two-factor keys are locked with it; without it you enter the mail password
  again and people set up two-factor login again.
