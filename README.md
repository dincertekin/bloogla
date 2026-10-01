# Bloogla

A lightweight, high-performance, single-binary CMS built with Rust and SQLite as a modern WordPress alternative.

---

## Features

- **Single Binary:** Server, admin panel, migrations, admin assets and the default theme compiled into one executable. Copy it to a server and run it.
- **Easy Setup:** Like WordPress, the first visit opens a setup page in the browser. Scripted installs can use `BLOOGLA_ADMIN_*` variables instead.
- **High Performance:** Compile-time HTML templates, sub-millisecond SQLite queries, zero JS heavy frameworks.
- **Content:** Posts with drafts and scheduling, standalone pages (About, Contact), tags, navigation menu, pagination, custom URL slugs with automatic redirects.
- **Editor:** Full-page Markdown editor with preview, image picker, cover images, keyboard saving, revision history and a local backup of unsaved text.
- **Search:** Full-text search (SQLite FTS5), accent-insensitive with prefix matching.
- **Analytics:** Privacy-friendly view counts and referring sites per day. No cookies or personal data; bots, link previews and your own visits are skipped.
- **Media Library:** Image uploads, resized for the web and stripped of location data.
- **SEO & RSS:** Meta descriptions, Open Graph and Twitter cards, JSON-LD, canonical URLs, full-content RSS, sitemap, `robots.txt` and a site icon.
- **Fast by default:** Settings are cached in memory and public pages answer repeat visits with `304 Not Modified`.
- **WordPress Import:** Bring posts, pages, categories, tags and featured images from a WordPress export; old WordPress links redirect to the new ones.
- **Operations:** Daily automatic backups, graceful shutdown, systemd/Caddy/nginx/Docker templates.
- **Security:** Argon2 hashing, session-based auth, CSRF protection, rate-limiting on login, security headers.

---

## Tech Stack

| Layer                       | Technology                    |
| --------------------------- | ----------------------------- |
| **Language**                | Rust                          |
| **Web Server**              | `axum` + `tokio`              |
| **Database**                | SQLite (`sqlx` with WAL mode) |
| **Templating**              | `askama` + `tera`             |
| **Frontend**                | HTMX + Vanilla CSS            |
| **Auth**                    | `argon2` + `tower-sessions`   |
| **Markdown & Sanitization** | `comrak` + `ammonia`          |

---

## Quick Start

### Development

```bash
cargo run

```

### Production Build

```bash
cargo build --release

```

Executables will be located at `./target/release/bloogla`.

### Configuration

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
| `BLOOGLA_BLOG_NAME`      | `My Blog`                | Site title for scripted setup.                                |

Bloogla keeps its files in the folder it is started from: `data/` (database and backups), `uploads/` and `themes/`.

### Commands

```bash
bloogla                          # start the server (same as `bloogla serve`)
bloogla backup [FILE]            # consistent copy of the live database
bloogla reset-password [EMAIL]   # set a new admin password and sign out all sessions
bloogla import-wordpress FILE    # import a WordPress export (.xml)
```

---

## Deploying to a VPS

On a Debian or Ubuntu server with your domain's DNS pointing at it, run as root:

```bash
curl -fsSL https://raw.githubusercontent.com/dincertekin/bloogla/main/deploy/install.sh | sh -s example.com
```

This downloads the latest release for your server. To install a binary you built yourself instead:

```bash
cargo build --release
scp target/release/bloogla deploy/install.sh root@your-server:
ssh root@your-server 'sh install.sh example.com'
```

The script installs Bloogla as a hardened systemd service in `/var/lib/bloogla` and sets up Caddy for automatic HTTPS. Then open your domain in a browser to create your account. Re-run the script with a new binary to upgrade.

Manual alternatives are in [`deploy/`](deploy/): a systemd unit, a Caddyfile and an nginx config.

**Docker:**

```bash
docker build -t bloogla .
docker run -d -p 8080:8080 -v bloogla:/app -e BLOOGLA_BASE_URL=https://example.com bloogla
# then open the site in your browser to finish setup
```

### Releasing

Push a version tag (`git tag v0.2.0 && git push --tags`). GitHub Actions builds static Linux binaries for x86-64 and ARM and attaches them to a release, which `install.sh` downloads.

## Moving from WordPress

1. In WordPress, go to **Tools → Export → All content** and download the `.xml` file.
2. Run `bloogla import-wordpress export.xml` (safe to re-run; existing posts are skipped).
3. Point your domain at Bloogla. Old links like `/2020/05/my-post/` redirect to `/post/my-post`.

Categories and tags both become tags. Images still load from the old site, so keep it online until you've re-uploaded them in **Admin → Media**.

---

## Security & Operations

- **CSRF:** Cross-site `POST`/`DELETE` requests are rejected using the browser's `Sec-Fetch-Site` header, falling back to an `Origin` check against `BLOOGLA_BASE_URL`.
- **Rate Limiting:** IP-based protection on `/admin/login` via `tower-governor`.
- **Visibility:** Drafts and not-yet-due scheduled posts are hidden from every public page, search, RSS and sitemap. Publish dates are UTC.
- **Backups:** Automatic daily backups in `data/backups/`; run `bloogla backup` for an on-demand copy. Restore by stopping Bloogla and replacing `data/bloogla.db` with a backup.
- **Uploads:** Files are checked by content (JPEG, PNG, GIF, WebP only), stored under random names, and photos are resized and stripped of metadata.

---

## Project Structure

```
.
├── Cargo.toml          # Dependencies and project settings
├── Dockerfile          # Container image
├── deploy/             # install.sh, systemd unit, Caddy and nginx configs
├── migrations/         # Embedded SQLite migrations
├── src/
│   ├── handlers/       # Axum route handlers (admin, auth, media, posts, public)
│   ├── assets.rs       # Files embedded in the binary (admin assets, bundled themes)
│   ├── backup.rs       # Database backups
│   ├── config.rs       # Environment configuration
│   ├── db.rs           # Database connections and setup
│   ├── import.rs       # WordPress import
│   ├── main.rs         # Commands and server startup
│   ├── models.rs       # Structs and database types
│   ├── seo.rs          # Meta, Open Graph and JSON-LD tags
│   ├── setup.rs        # First-run setup (browser or environment variables)
│   ├── tags.rs         # Tag queries
│   ├── templates.rs    # Askama admin-panel template structs
│   ├── themes.rs       # Theme discovery
│   └── utils.rs        # Markdown, slugs, dates and other helpers
├── static/             # Admin CSS and JS (embedded)
├── templates/          # Askama admin-panel templates
└── themes/default/     # The bundled theme (embedded, installed on first start)
```

Created at runtime: `data/` (database, backups), `uploads/`.

## Writing Themes

A theme is a folder in `themes/` with a `theme.toml` and Tera templates in `templates/`:

| Template     | Used for                                  | Required |
| ------------ | ----------------------------------------- | -------- |
| `index.html` | Home page and search results              | Yes      |
| `post.html`  | Single post                               | Yes      |
| `tag.html`   | Posts with a tag                          | Yes      |
| `page.html`  | Standalone pages (falls back to `post.html`) | No    |
| `404.html`   | Not found page                            | No       |

Every template gets `blog_name`, `blog_description`, `base_url`, `menu` (list of `label`/`url`), `tags`, `search_query`, `show_views` and `seo_head`. Put `{{ seo_head | safe }}` inside `<head>`. Listings also get `posts` and `pagination` (`current`, `total_pages`, `prev_url`, `next_url`); single pages get `post` and `content_html`.
