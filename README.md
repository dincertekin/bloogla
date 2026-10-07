# Bloogla

A lightweight, high-performance, single-binary CMS built with Rust and SQLite as a modern WordPress alternative.

---

## Features

- **Single Binary:** Server, admin panel, migrations, admin assets and the default theme compiled into one executable. Copy it to a server and run it.
- **Easy Setup:** Like WordPress, you create your account in the browser: Bloogla prints a one-time setup link when it starts (the installer shows it), so nobody else can claim a fresh site first. Scripted installs can use `BLOOGLA_ADMIN_*` variables instead.
- **High Performance:** Compile-time HTML templates, sub-millisecond SQLite queries, zero JS heavy frameworks.
- **Content:** Posts with drafts and scheduling, standalone pages (About, Contact), tags, navigation menu, pagination, custom URL slugs with automatic redirects.
- **Editor:** Visual editor (bold looks bold as you type) with a preview; posts are saved as Markdown, and HTML it can't show (like video embeds) is kept as written. Image picker, cover images, keyboard saving, revision history and a local backup of unsaved text.
- **Search:** Full-text search (SQLite FTS5), accent-insensitive with prefix matching.
- **People:** Admin, editor and author roles, bylines, and each person's own profile and password.
- **Languages:** English, Deutsch, Español, Français, Italiano, Português, Türkçe, Русский, 日本語 and 中文 for the admin panel, the theme and emails. The site has a language, and each person can pick their own for the admin.
- **Email & newsletter:** Comment notifications and a double opt-in newsletter over your own SMTP server, with one-click unsubscribe.
- **Shortcodes:** `[youtube URL]`, `[vimeo URL]`, `[audio URL]`, `[video URL]`, `[toc]`, plus your theme's own.
- **Custom fields:** Extra values per post (location, rating…) for themes and the API.
- **Webmentions:** Notify blogs you link to, and receive their mentions in the comment queue.
- **Comments:** Optional, with approval queue and quiet spam protection (no captchas, no tracking).
- **JSON API:** Public read endpoints and token-based publishing for apps and scripts.
- **Analytics:** Privacy-friendly view counts and referring sites per day. No cookies or personal data; bots, link previews and your own visits are skipped.
- **Media Library:** Image uploads, resized for the web and stripped of location data.
- **SEO & RSS:** Meta descriptions, Open Graph and Twitter cards, JSON-LD, canonical URLs, full-content RSS, sitemap, `robots.txt` and a site icon.
- **Fast by default:** Settings are cached in memory and public pages answer repeat visits with `304 Not Modified`.
- **WordPress Import:** Bring posts, pages, categories, tags and featured images from a WordPress export; old WordPress links redirect to the new ones.
- **Operations:** Daily automatic backups, graceful shutdown, systemd/Caddy/nginx/Docker templates.
- **Security:** Argon2 hashing, optional two-factor login, encrypted secrets, CSRF protection, login rate limits, strict Content Security Policy, security headers.

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
# then open the setup link it prints (http://localhost:8080/setup?code=...)

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
| `BLOOGLA_TLS_DOMAINS`    |                          | Comma-separated domains. Turns on built-in HTTPS with automatic Let's Encrypt certificates (port 443, and 80 redirects). |
| `BLOOGLA_TLS_EMAIL`      |                          | Contact address for certificate expiry notices.               |
| `BLOOGLA_TLS_STAGING`    | `false`                  | Use Let's Encrypt's test server while trying things out.      |
| `BLOOGLA_HTTP_PORT`      | `80`                     | Port that redirects to HTTPS when built-in HTTPS is on.       |
| `BLOOGLA_LOG`            | `info`                   | Log level filter, e.g. `debug` or `warn`.                     |
| `BLOOGLA_LOG_FORMAT`     | text                     | `json` for one JSON object per line.                          |
| `BLOOGLA_ADMIN_NAME`     |                          | Display name for scripted setup.                              |
| `BLOOGLA_WEBMENTION_ALLOW_LOCAL` | `false`          | Development only: let webmentions fetch local addresses (normally blocked). |

Bloogla keeps its files in the folder it is started from: `data/` (database and backups), `uploads/` and `themes/`.

### Commands

```bash
bloogla                          # start the server (same as `bloogla serve`)
bloogla backup [FILE]            # consistent copy of the live database
bloogla reset-password [EMAIL]   # set a new admin password and sign out all sessions
bloogla import-wordpress FILE    # import a WordPress export (.xml)
bloogla disable-2fa EMAIL        # turn off two-factor login for someone locked out
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

The script installs Bloogla as a hardened systemd service in `/var/lib/bloogla` and sets up Caddy for automatic HTTPS. Without Caddy (or with `HTTPS=builtin`), Bloogla gets its own certificates instead. At the end it prints a one-time setup link; open it to create your account. Re-run the script with a new binary to upgrade.

Manual alternatives are in [`deploy/`](deploy/): a systemd unit, a Caddyfile and an nginx config.

**Docker:**

```bash
docker build -t bloogla .
docker run -d -p 8080:8080 -v bloogla:/app -e BLOOGLA_BASE_URL=https://example.com bloogla
# then open the setup link from `docker logs <container>` to create your account
```

### Releasing

Push a version tag (`git tag v0.2.0 && git push --tags`). GitHub Actions builds static Linux binaries for x86-64 and ARM and attaches them to a release, which `install.sh` downloads.

## JSON API

Reading needs no authentication and is open to other sites (CORS):

```bash
curl https://example.com/api/posts?page=1&per_page=10&tag=travel
curl https://example.com/api/posts/my-post      # includes Markdown and rendered HTML
curl https://example.com/api/pages/about
curl https://example.com/api/tags
```

To write, create a token in **Admin → Profile → API tokens**. It acts with your role, so an author's token can only change their own posts.

```bash
TOKEN=bl_...
curl https://example.com/api/me -H "Authorization: Bearer $TOKEN"

curl -X POST https://example.com/api/posts -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"title": "Hello", "content": "Written with **Markdown**.", "status": "published", "tags": ["notes"]}'

# Fields you leave out keep their current values
curl -X PUT https://example.com/api/posts/42 -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" -d '{"title": "Hello again"}'

curl -X DELETE https://example.com/api/posts/42 -H "Authorization: Bearer $TOKEN"
```

Post fields: `title`, `content` (Markdown), `slug`, `status` (`draft`, `published`, `scheduled`), `published_at` (UTC, e.g. `2026-10-02T09:00:00`), `cover_image`, `tags` (names), and `page: true` to create a page. Errors come back as `{"error": "..."}` with a matching status code.

## Moving from WordPress

1. In WordPress, go to **Tools → Export → All content** and download the `.xml` file.
2. Run `bloogla import-wordpress export.xml` (safe to re-run; existing posts are skipped).
3. Point your domain at Bloogla. Old links like `/2020/05/my-post/` redirect to `/post/my-post`.

Categories and tags both become tags. Images still load from the old site, so keep it online until you've re-uploaded them in **Admin → Media**.

---

## Security & Operations

- **Content Security Policy:** The admin panel, login and setup only run scripts from Bloogla's own files (no inline scripts, no `eval`), so injected HTML can't run code. Admin JavaScript lives in `admin/static/js/`; a test fails if inline scripts return to the templates.
- **CSRF:** Cross-site `POST`/`DELETE` requests are rejected using the browser's `Sec-Fetch-Site` header, falling back to an `Origin` check against `BLOOGLA_BASE_URL`.
- **Rate Limiting:** Per-address limits on login, comments, newsletter sign-ups and webmentions. After 5 failed sign-ins in 15 minutes, an account accepts one attempt per minute from anywhere, which stops guessing spread across many addresses (accounts are slowed, never locked, so nobody can lock the owner out). Behind nginx, use the included config: it overwrites `X-Forwarded-For` so visitors can't fake their address.
- **Visibility:** Drafts and not-yet-due scheduled posts are hidden from every public page, search, RSS and sitemap. Publish dates are UTC.
- **Backups:** Automatic daily backups in `data/backups/`; run `bloogla backup` for an on-demand copy. Restore by stopping Bloogla and replacing `data/bloogla.db` with a backup.
- **Uploads:** Files are checked by content (JPEG, PNG, GIF, WebP only), stored under random names, and photos are resized, stripped of metadata, and get an 800px-wide copy for phones.
- **Themes:** Only files in a theme's `static/` folder are public; templates and `theme.toml` are not served. Only admins can upload themes; uploads are checked for unsafe paths, allowed file types and size (20 MB, 100 MB unpacked) before anything is installed, and are logged as security events.
- **Two-factor login (optional, recommended):** Turn it on in Profile with any authenticator app; you get 10 one-time recovery codes. Nobody is forced to use it. Admins can turn it off for someone who lost their phone, and `bloogla disable-2fa EMAIL` works on the server.
- **Stored secrets:** The mail server password and two-factor keys are encrypted with a key in `data/secret.key`. Keep that file with your backups; without it you re-enter the mail password and set up two-factor login again.
- **Setup:** A fresh site can only be set up with the one-time code from the setup link Bloogla prints at start.
- **Passwords:** At least 12 characters with a lowercase letter, an uppercase letter, a number and a symbol. Forms show what's missing as you type, and the server checks again. Rules live in `src/app/security.rs`.
- **Sessions:** HTTP-only, same-site cookies (HTTPS-only in production); expired sessions are removed hourly. Changing a password (or an admin resetting it) signs that account out on every other device.
- **Security log:** Sign-ins, failed sign-ins, password and role changes, new people and API tokens are logged as `security event=...` lines. On a server: `journalctl -u bloogla | grep "security event"`.
- **Dependencies:** CI runs `cargo audit` on every push and weekly, and Dependabot proposes updates. `install.sh` checks the downloaded binary against its published checksum.

### Planned

- **Content Security Policy for custom themes:** themes can already add sources in `theme.toml`; a settings screen for this may follow.

---

## Project Structure

```
.
├── Cargo.toml              # Dependencies, grouped by what they're for
├── askama.toml             # Tells Askama where the admin templates are
├── admin/                  # The admin panel's look (compiled into the binary)
│   ├── templates/          #   HTML pages (Askama)
│   ├── static/             #   CSS and JS, served at /static
│   └── visual-editor/      #   Source of the visual editor (built into static/js/visual-editor.js)
├── themes/default/         # The bundled public theme (written to themes/ on first start)
├── migrations/             # Database tables, applied automatically on start
├── deploy/                 # install.sh, systemd unit, Caddy and nginx configs
└── src/
    ├── main.rs             # Start here: reads the command and runs it
    ├── app/                # Building blocks used everywhere
    │   ├── config.rs       #   BLOOGLA_* environment variables
    │   ├── state.rs        #   AppState: what every request handler can reach
    │   ├── models.rs       #   Shared data types: Post, Tag, Role, CurrentUser...
    │   └── security.rs     #   Passwords, random tokens, API token hashing
    ├── commands/           # The command line: serve, backup, reset-password, import-wordpress
    ├── server/             # Running the website
    │   ├── mod.rs          #   Starting the server, sessions, shutdown
    │   ├── routes.rs       #   Every URL and the handler that answers it
    │   ├── middleware.rs   #   CSRF, sign-in and role checks, ETags, logging...
    │   ├── assets.rs       #   Serving the embedded admin CSS/JS
    │   └── tls.rs          #   Built-in HTTPS
    ├── handlers/           # The code behind each URL
    │   ├── site/           #   Public website: pages, feeds, comments, newsletter
    │   ├── admin/          #   Admin panel: one file per screen
    │   └── api.rs          #   JSON API
    ├── db/                 # Reading and writing the database
    │   ├── settings.rs     #   Every site setting and its default
    │   ├── posts.rs        #   Finding, saving and deleting posts and pages
    │   └── tags.rs, fields.rs, users.rs
    ├── content/            # Turning Markdown into web pages
    │   ├── markdown.rs     #   Markdown → safe HTML, excerpts
    │   ├── shortcodes.rs   #   [youtube], [toc] and theme shortcodes
    │   ├── seo.rs          #   Meta, Open Graph and JSON-LD tags
    │   └── text.rs         #   Slugs, escaping, dates
    ├── services/           # Work besides answering pages
    │   ├── email.rs        #   Sending email over SMTP (notifications, newsletter)
    │   ├── webmention.rs   #   Sending and verifying webmentions
    │   ├── backup.rs       #   Database backups
    │   ├── themes.rs       #   Finding, installing and rendering themes
    │   └── wordpress_import.rs
    └── i18n/               # Interface languages: one file per language (de.rs, fr.rs...)
```

Created at runtime: `data/` (database, backups), `uploads/`.

### Adding something new

- **A new page or endpoint:** write a handler in `src/handlers/`, then add one `.route(...)` line in `src/server/routes.rs`.
- **A new admin screen:** add `admin/templates/<name>.html` and `src/handlers/admin/<name>.rs` (the template struct sits next to its handlers), then add the route in the right role group.
- **A new setting:** add a field to `Settings` in `src/db/settings.rs` and give it a default there.
- **A database change:** add a new file to `migrations/` (never edit one that has already run).
- **New interface text:** wrap it in `me.t("...")` and add a translation to each file in `src/i18n/`; `cargo test` lists anything missing.
- **A new language:** copy `src/i18n/de.rs` to `<code>.rs`, translate it, and add one line to `LANGUAGES` in `src/i18n/mod.rs`.

Before sending changes: `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test` (CI runs the same).

## Writing Themes

A theme is a folder in `themes/` with a `theme.toml` and Tera templates in `templates/`. The easiest start is a copy of `themes/default` with a new folder name. Install a theme by uploading it as a .zip in **Admin → Themes** (the files at the top of the .zip or inside one folder; the folder or file name becomes the theme's name), or by copying its folder into `themes/` on the server.

```toml
name = "My Theme"
version = "1.0.0"
author = "Your Name"
description = "One sentence about it."
preview_image = "static/preview.png"   # optional, shown in Admin → Themes
```


| Template       | Used for                                           | Required |
| -------------- | -------------------------------------------------- | -------- |
| `index.html`   | Home page and search results                       | Yes      |
| `post.html`    | Single post                                        | Yes      |
| `tag.html`     | Posts with a tag                                   | Yes      |
| `page.html`    | Standalone pages (falls back to `post.html`)       | No       |
| `404.html`     | Not found page                                     | No       |
| `message.html` | Short notices (newsletter confirm, unsubscribe)    | Yes      |

Templates refer to each other by their path inside the theme, so a copied theme works under any name: `{% extends "templates/layout.html" %}`, `{% include "templates/post_card.html" %}`.

A theme with a mistake (a template that doesn't parse, a missing required template, a broken `theme.toml`) is never used: Admin → Themes shows what's wrong, and the site keeps using its current theme. Uploads are checked the same way before they're installed, and may only contain templates, styles, scripts, images and fonts.

Every template gets `blog_name`, `blog_description`, `base_url`, `lang` (e.g. `tr`), `t` (translated interface text, e.g. `{{ t.back_to_all_posts }}`), `menu` (list of `label`/`url`), `tags`, `search_query`, `show_views`, `newsletter_enabled`, `seo_head` and `asset_version`. Put `{{ seo_head | safe }}` inside `<head>`.

Files in the theme's `static/` folder are served at `/theme-assets/<theme>/static/...`. Add `?v={{ asset_version }}` (the theme's version) to those URLs: browsers then keep them for a year, and a new theme version is picked up at once.

- Listings also get `posts` and `pagination` (`current`, `total_pages`, `prev_url`, `next_url`).
- Single pages get `post` (with `post.fields.<name>` for custom fields, `post.author_name`, and `post.cover_width`/`post.cover_height` for images from the media library, and `post.cover_small`, an 800px-wide copy for `srcset`) and `content_html`.
- Posts with comments on also get `comments_enabled`, `comments`, `comment_action` and `comment_notice`; see the default theme's `comments.html` and `subscribe.html`.

### Scripts and the security policy

Theme pages are sent with a Content Security Policy: scripts, styles and fonts load from the site itself, images and media from any HTTPS address, and embeds from YouTube and Vimeo. Inline `<script>` blocks and `onclick=` attributes don't run, so put JavaScript in files under `static/js/`. If a theme needs more (web fonts, for example), list the extra sources in `theme.toml`:

```toml
[csp]
style-src = ["https://fonts.googleapis.com"]
font-src = ["https://fonts.gstatic.com"]
```

The default theme loads its syntax highlighter (`static/js/prism.js`, with about 20 common languages) only on pages that contain code.

### Theme shortcodes

Add `shortcodes/<name>.html` to a theme to make `[name ...]` available in posts. The template gets `args` (positional values) and `params` (`key=value` pairs). The default theme's `shortcodes/note.html` turns `[note "Text" kind="warning"]` into a highlighted box.

### Updating the bundled theme

Bump `version` in `theme.toml`. On start, Bloogla replaces an older installed copy and keeps the previous one in `data/theme-backups/`. Themes replaced by an upload or deleted in the admin are kept there too.
