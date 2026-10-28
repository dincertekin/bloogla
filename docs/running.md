# Running Bloogla

How to run Bloogla, update it and keep your site safe.

- [On your computer](#on-your-computer)
- [On a server](#on-a-server)
- [Settings before setup](#configuration)
- [Commands](#commands)
- [Updating](#updating)
- [Backups](#backups)
- [Moving from WordPress](#moving-from-wordpress-and-exporting)

Bloogla keeps everything in the folder it starts from: `data/` (database and
backups), `uploads/` (images and videos) and `themes/`.

## On your computer

**Windows:** download `bloogla-x86_64-windows.exe` from the
[latest release](https://github.com/dincertekin/bloogla/releases/latest),
put it in its own folder (for example `Documents\Bloogla`) and open it. It
opens the setup page in your browser. Keep the window open while you use
your site.

The first time, Windows says "Windows protected your PC": click **More
info**, then **Run anyway**. If it asks about the network, choose **Private
networks** and **Allow**.

**Mac:** there's no download. Use Docker Desktop (see below), or install
[Rust](https://rustup.rs) and run `cargo build --release` in a copy of this
repository.

## On a server

Point your domain at the server first, and open ports 80 and 443.

**With Docker:**

```bash
docker run -d --name bloogla --restart unless-stopped -p 80:80 -p 443:443 \
  -v bloogla:/app -e BLOOGLA_TLS_DOMAINS=example.com ghcr.io/dincertekin/bloogla
docker logs bloogla   # open the setup link it prints
```

**Without Docker:** download `bloogla-x86_64-linux` (or `bloogla-aarch64-linux`
for ARM servers), rename it to `bloogla`, and start it in an empty folder:

```bash
chmod +x bloogla
BLOOGLA_TLS_DOMAINS=example.com ./bloogla   # open the setup link it prints
```

Keep it running with a systemd service or `tmux`. Bloogla gets its HTTPS
certificate from Let's Encrypt by itself.

To try it without a domain, leave out `BLOOGLA_TLS_DOMAINS` and open
`http://localhost:8080`.

## Configuration

Everything is set in the admin panel. These few settings are needed before
it exists:

| Variable              | Default                  | What it does |
| --------------------- | ------------------------ | ------------ |
| `BLOOGLA_TLS_DOMAINS` |                          | Your domains, comma-separated. Turns on HTTPS (port 443; port 80 redirects). |
| `BLOOGLA_TLS_EMAIL`   |                          | Where Let's Encrypt sends certificate notices. |
| `BLOOGLA_TLS_STAGING` | `false`                  | Use Let's Encrypt's test server while you try things. |
| `BLOOGLA_HTTP_PORT`   | `80`                     | The port that redirects to HTTPS. |
| `BLOOGLA_HOST`        | `0.0.0.0`                | Address to listen on. `127.0.0.1` behind a reverse proxy. |
| `BLOOGLA_PORT`        | `8080`                   | Port to listen on (without HTTPS). |
| `BLOOGLA_BASE_URL`    | `http://localhost:$PORT` | Your site's address, behind a reverse proxy. |
| `BLOOGLA_PRODUCTION`  | `false`                  | `true` behind a reverse proxy with HTTPS: secure cookies and HSTS. |
| `BLOOGLA_LOG`         | `info`                   | How much to log: `debug`, `info` or `warn`. |
| `BLOOGLA_LOG_FORMAT`  | text                     | `json` for one JSON object per line. |

## Commands

Only for getting back in when you can't sign in:

```bash
bloogla reset-password [EMAIL]   # set a new password (the owner's, without EMAIL)
bloogla disable-2fa EMAIL        # turn off two-factor login for someone who lost their phone
```

## Updating

Download a backup first (**Settings → Backup**). Then:

- **Linux:** **Settings → Updates → Install**. Bloogla downloads the new
  version, checks its signature and restarts in a few seconds. The old
  program stays as `bloogla.old`; rename it back to undo.
- **Docker:** `docker pull ghcr.io/dincertekin/bloogla`, then
  `docker rm -f bloogla` and run the same `docker run` command again. Your
  site is in the `bloogla` volume.
- **Windows:** close Bloogla, replace the `.exe` with the new one, and open it.
- **Mac:** build the new version again.

Bloogla only asks GitHub for new versions if you tick **Check for new
versions every day** in Settings → Updates. It can also install them by
itself (**Install new versions automatically**). Both are off at first.

## Backups

- **Download a backup:** **Settings → Backup** gives one `.zip` with your
  database, images and videos.
- **Restore:** in **Settings → Backup → Restore**, choose a `.zip` and click
  **Check backup** to see what's in it. **Restore this backup** saves your
  site as it is now in `data/backups/` first, then replaces everything.
- **Daily copies:** Bloogla keeps the last 7 days of the database in
  `data/backups/`. Turn this off in **Settings → Backup**.
- **Keep `data/secret.key` with your backups.** Without it, you type your
  mail password again and everyone sets up two-factor login again.

If Bloogla won't start: stop it, unpack the backup `.zip` into its folder,
delete `data/bloogla.db-wal` and `data/bloogla.db-shm` if they're there, and
start it again.

## Moving from WordPress, and exporting

Both are in **Settings → Import & export**.

**Import from WordPress:**

1. In WordPress, go to **Tools → Export**, choose **All content** and download the file.
2. In Bloogla, choose the file and click **Check file**. You see what's in it before anything changes.
3. Click **Import**. Bloogla saves a backup first, then imports in the background.

What comes along: posts, pages, drafts, tags (categories become tags),
approved comments, and the images on your old site (copied into Media).
People are matched by email; posts by someone who isn't here go to you.

Old addresses keep working: `/2024/05/lemon-cake/` leads to
`/post/lemon-cake`. For other old addresses, add them in **Settings →
Redirects**.

**Export:** **Download WordPress file** works with WordPress and most blog
tools. **Download Markdown files** gives one text file per post and page,
with the images. To copy the whole site, use Backup instead.
