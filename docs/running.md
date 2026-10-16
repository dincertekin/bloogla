# Running Bloogla

Everything about putting Bloogla on a server and keeping it running. New
here? The [README](../README.md#quick-start) has the short version.

- [On your computer](#on-your-computer)
- [On a server](#on-a-server)
- [Configuration](#configuration)
- [Commands](#commands)
- [Updating](#updating)
- [Backups](#backups)
- [Moving from WordPress, and exporting](#moving-from-wordpress-and-exporting)

## On your computer

On Windows, download `bloogla-x86_64-windows.exe` from the
[latest release](https://github.com/dincertekin/bloogla/releases/latest).
Open it: a window shows your site's address and opens the
setup page in your browser. Keep that window open while you use your site;
close it (or press Ctrl+C) to stop.

Bloogla isn't signed by Microsoft yet, so the first time Windows says
"Windows protected your PC": click **More info**, then **Run anyway**. If
Windows asks whether Bloogla may use the network, choose **Private networks**
and **Allow**.

Bloogla keeps your site in the folder it starts from: `data/` (the database
and backups), `uploads/` (pictures) and `themes/`. To keep things tidy, make
a folder for it (for example `Bloogla` in Documents) and put the program
there before opening it.

**On a Mac** there's no ready-made download (releases stopped including one
in 1.2.0). Use Docker Desktop with the commands under "On a server", or
build it with [Rust](https://rustup.rs): `cargo build --release` in a copy of
this repository makes `target/release/bloogla`. To update a Mac build, build
it again from the new version.

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
| `BLOOGLA_TLS_DOMAINS`    |                          | Comma-separated domains. Turns on built-in HTTPS with automatic Let's Encrypt certificates (port 443, and 80 redirects). |
| `BLOOGLA_TLS_EMAIL`      |                          | Contact address for certificate expiry notices.               |
| `BLOOGLA_TLS_STAGING`    | `false`                  | Use Let's Encrypt's test server while trying things out.      |
| `BLOOGLA_HTTP_PORT`      | `80`                     | Port that redirects to HTTPS when built-in HTTPS is on.       |
| `BLOOGLA_LOG`            | `info`                   | Log level filter, e.g. `debug` or `warn`.                     |
| `BLOOGLA_LOG_FORMAT`     | text                     | `json` for one JSON object per line.                          |

Bloogla keeps its files in the folder it is started from: `data/` (database and backups), `uploads/` and `themes/`.

## Commands

```bash
bloogla                          # start the website
bloogla reset-password [EMAIL]   # set a new admin password and sign out all sessions
bloogla disable-2fa EMAIL        # turn off two-factor login for someone locked out
```

Everything else is done in the admin panel; these two commands are for
getting back in when you can't sign in.

## Updating

**Settings → Updates** shows your version and has **Check for updates**, which asks GitHub for the newest release. New versions also show on the Dashboard, with a badge next to it in the menu. Two checkboxes there, both off at first: **Check for new versions every day**, and **Install new versions automatically**. Bloogla never checks on its own unless the first one is ticked. GitHub only learns your server's address and Bloogla's version.

How a new version gets installed depends on how Bloogla runs (download a backup first, in **Settings → Backup**):

- **The binary on Linux:** click **Install Bloogla 1.2.0** (or let the automatic install do it). Bloogla downloads the new program, checks that it's signed with Bloogla's release key (anything else is refused), swaps it in and restarts in place; the site is down for a few seconds. The previous program stays next to it as `bloogla.old`: to go back, stop Bloogla and rename it to `bloogla`. Bloogla must be allowed to write to the folder its program is in.
- **Docker:** a container can't replace itself, so: `docker pull ghcr.io/dincertekin/bloogla`, remove the container (`docker rm -f bloogla`) and run the same `docker run` command again. Your site lives in the `bloogla` volume, so nothing is lost.
- **Macs:** like the Linux binary, **Install** in Settings → Updates downloads, checks and restarts Bloogla.
- **Windows** (or a folder Bloogla can't write to): stop Bloogla, download the new `.exe` from the [latest release](https://github.com/dincertekin/bloogla/releases/latest), put it in place of the old one and start it again from the same folder.

The database is upgraded automatically on start.

## Backups

- **Download everything:** admins get the database and all images as one
  `.zip` in **Settings → Backup**.
- **Restore:** in **Settings → Backup → Restore**, choose a backup `.zip` and
  click **Check backup**: Bloogla shows what's in it (the newest post, how
  many posts, pages, people and images) before anything changes. **Restore
  this backup** first saves your site as it is now in
  `data/backups/before-restore-….zip`, then restarts with the backup in
  place. Everything on the site is replaced, images included. Backups from
  every Bloogla version so far can be restored; one from a newer version
  asks you to update first. On Windows, close Bloogla and open it again to
  finish. If Bloogla can't start at all, you can also restore by hand: stop
  it, unpack the `.zip` into its folder (in a backup from 1.0.1 or earlier,
  move `bloogla.db` into `data/`), delete `data/bloogla.db-wal` and
  `data/bloogla.db-shm` if they exist, and start it again.
- **On the server:** Bloogla also keeps a daily copy of the database in
  `data/backups/` (the last 7). It's on at first; turn it off in
  **Settings → Backup**. If you turned it off with `BLOOGLA_AUTO_BACKUP=false`
  in Bloogla 1.0, it stays off after updating.
- **Keep `data/secret.key` with your backups.** The mail server password and
  two-factor keys are locked with it; without it you enter the mail password
  again and people set up two-factor login again.

## Moving from WordPress, and exporting

Both are in **Settings → Import & export** (admins only).

- **Import from WordPress:** in WordPress, use **Tools → Export**, choose
  "All content" and download the file. In Bloogla, choose it and click
  **Check file**: you see how many posts, pages, tags, comments and images
  it has, who wrote them, and which posts are skipped because a post here
  already has the same address, before anything changes.
  - People are matched by email. Posts by someone who isn't on the site go
    to you; add them in **People** first to keep their name on their posts.
  - Drafts stay drafts, scheduled posts keep their date, and WordPress
    categories become tags. Only approved comments come along (if you tick
    the box); pingbacks, comments waiting for approval and spam stay behind.
  - **Copy the images into Bloogla** (on at first) downloads the old site's
    images into **Media** and points the posts at the copies. Only images on
    the old site's own address are copied, and only from public internet
    addresses; ones on other sites, or that fail, stay where they were and
    are listed at the end with the reason.
  - **Import** first saves a backup of your site as it is now in
    `data/backups/before-import-….zip`, then runs in the background (you can
    leave the page). Nothing that's already on the site is changed.
  - Old addresses keep working once your domain points to Bloogla:
    `/2024/05/lemon-cake/` and `/lemon-cake/` lead to `/post/lemon-cake`, and
    `/category/desserts/` to `/tag/desserts`.
- **Export:** **Download WordPress file** gives a file WordPress (Tools →
  Import → WordPress) and most blog tools can import, with every post and
  page (drafts included), their tags, approved comments and images (WordPress
  copies them from your site while it's online). **Download Markdown files**
  gives a `.zip` with `posts/<address>.md`, `pages/<address>.md` (each with
  its title, date, tags and cover at the top) and the images in `uploads/`.
  To keep a copy of the whole site, use Backup instead.
