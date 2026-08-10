# ⚡ Bloogla

**Bloogla** is a self-hosted content management system (CMS) built in Rust. It ships as a single binary, runs with minimal resource usage, and stays fast even on cheap VPS hardware. It's built as a lightweight alternative to heavier systems like WordPress.

---

## 🎯 Core Principles

- **Performance first:** Compile-time HTML templating, sub-millisecond local SQLite queries, and lightweight static assets.
- **Security by default:** Argon2 password hashing, secure cookie-based sessions, and parameterized SQL queries (no SQL injection surface).
- **Minimal footprint:** No heavy JS frameworks (React, Vue). HTMX and plain CSS handle dynamic behavior instead.
- **Single binary:** Server, templates, database migrations, and admin panel all compile into one executable (`./bloogla`).

---

## 🛠️ Tech Stack

| Component | Technology | Why |
| :--- | :--- | :--- |
| **Language** | Rust | Zero-cost abstractions, memory safety, and raw speed. |
| **Web server** | `axum` + `tokio` | Async, high-throughput HTTP server. |
| **Database** | SQLite (`sqlx`) | Local file access, no network latency (WAL mode). |
| **Templating** | `askama` | Compiles HTML templates into Rust code at build time. |
| **Frontend interactivity** | HTMX | AJAX-style HTML updates without a JS bundle. |
| **Auth & sessions** | `argon2` + `tower-sessions` | Password hashing and session management. |
| **Markdown parsing** | `comrak` / `pulldown-cmark` | Converts content to sanitized HTML. |

---

## ✅ Implemented Features

- [x] **CLI first-run wizard:** On first launch, collects blog title, admin email, password, and port over the terminal, then writes the config file.
- [x] **Database & migrations:** Embedded SQL migrations, SQLite running in WAL mode.
- [x] **Auth & sessions:** Argon2 password verification and `tower-sessions`-backed auth guarding `/admin` routes.
- [x] **HTMX-powered admin panel:** Add, edit, and delete content without full page reloads.
- [x] **SEO & syndication:**
  - Reading time estimation.
  - Dynamic **RSS 2.0** feed (`/rss.xml`).
  - Dynamic **sitemap** (`/sitemap.xml`).
  - HTML sanitization (`ammonia`) for excerpt generation.

---

## 🚫 Out of Scope (for now)

- **Plugin architecture:** Left out to avoid the performance and complexity cost of a plugin system.
- **Multi-user / role management:** The system is designed around a single admin account.
- **External database support (PostgreSQL/MySQL):** SQLite keeps the zero-config, single-binary model intact without adding a network dependency.
- **Live theme editor:** Templates are compiled at build time with Askama instead of rendered from disk, trading flexibility for performance.

---

## 🛡️ Production & Security Architecturew

- **CSRF protection:** Uses the Double Submit Cookie pattern, enforced at the Axum middleware level. HTMX requests carry the `X-CSRF-Token` header automatically via `hx-headers`. Chosen over the Synchronizer Token pattern since it needs no server-side token store, which fits the single-admin, mostly-stateless design.
- **Rate limiting:** `/admin/login` is rate-limited per IP with `tower-governor` to guard against brute-force and CPU-exhaustion DoS attacks. If Bloogla runs behind a reverse proxy (nginx, Caddy), the proxy must be configured to forward the real client IP via `X-Forwarded-For`, and `tower-governor` must be configured to trust and read it. Without this, rate limiting will apply to the proxy's IP instead of individual clients.
- **Media uploads:** Handled via `axum::extract::Multipart`, stored locally under `./uploads` by default and served through Axum's static file handler. To prevent path traversal, uploaded filenames are never used directly on disk; each file is renamed to a generated UUID plus its validated extension. Uploads are restricted by a MIME type whitelist (e.g. `image/jpeg`, `image/png`) and a maximum file size. Optional S3-compatible storage (MinIO, R2, AWS S3) is planned for offloading/backup.
- **Backup:** WAL mode alone does not make `bloogla.db` safe to copy directly. Because writes may still sit in the `-wal` file, a raw `cp` can produce an inconsistent snapshot. Backups should instead use SQLite's `VACUUM INTO 'backup.db'` command (or the `sqlite3 .backup` CLI command), both of which produce an atomic, consistent copy while the app keeps running.

---

## 🚀 Quick Start

### Run in development mode

```bash
cargo run
```

### Build for production

For the smallest, optimized binary:

```bash
cargo build --release
```

Copy the compiled `./target/release/bloogla` executable to your server and run it directly.
