# Bloogla

A lightweight, high-performance, single-binary CMS built with Rust and SQLite as a modern WordPress alternative.

---

## Features

- **Single Binary:** Server, templates, migrations, and admin panel compiled into one executable (`./bloogla`).
- **High Performance:** Compile-time HTML templates, sub-millisecond SQLite queries, zero JS heavy frameworks.
- **CLI Setup Wizard:** Interactive terminal setup for blog configuration on first run.
- **Admin Panel:** HTMX-powered dashboard for content management without full page reloads.
- **SEO & RSS:** Built-in RSS feed (`/rss.xml`), sitemap (`/sitemap.xml`), and reading time estimation.
- **Security:** Argon2 hashing, session-based auth, CSRF protection, rate-limiting on login, and secure media uploads.

---

## Tech Stack

| Layer                       | Technology                    |
| --------------------------- | ----------------------------- |
| **Language**                | Rust                          |
| **Web Server**              | `axum` + `tokio`              |
| **Database**                | SQLite (`sqlx` with WAL mode) |
| **Templating**              | `askama`                      |
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

---

## Security & Operations

- **CSRF:** Double Submit Cookie pattern enforced via Axum middleware.
- **Rate Limiting:** IP-based protection on `/admin/login` via `tower-governor`.
- **Media Uploads:** UUID-renamed files stored in `./uploads` with MIME/size validation.
- **Backups:** Use `VACUUM INTO 'backup.db'` or `sqlite3 .backup` for atomic hot backups.

---

## Project Structure

```
.
├── Cargo.toml          # Dependencies and project settings
├── Dockerfile          # Production container setup
├── docker-compose.yml  # Deployment configuration
├── migrations/         # Embedded SQLite migrations
├── src/                # Application source code
│   ├── handlers/       # Axum route handlers (auth, admin, public)
│   ├── config.rs       # App configuration loader
│   ├── db.rs           # Database connections and setup
│   ├── main.rs         # Entry point and server initialization
│   ├── models.rs       # Structs and database types
│   ├── tags.rs         # Custom Askama template tags/helpers
│   ├── templates.rs    # Askama template structs
│   └── utils.rs        # Auth, hashing, and helper utilities
├── static/             # Static assets (CSS, JS, images)
└── templates/          # Askama HTML templates
```
