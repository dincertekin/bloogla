# Contributing to Bloogla

Thanks for helping! Bloogla is for people who want a website without being
developers, so every change is judged by one question: **does it make
Bloogla simpler, faster or safer for them?**

## Ways to help

- **Report a bug or suggest an idea:** [open an issue](https://github.com/dincertekin/bloogla/issues/new/choose).
  Security problems go [privately](SECURITY.md) instead.
- **Translate:** the interface is in English and Turkish. A new language is
  one file (see [Adding something new](#adding-something-new)), and fixes to
  existing translations are very welcome.
- **Make a theme:** themes are HTML templates and CSS, no Rust needed. See
  [docs/themes.md](docs/themes.md).
- **Write code:** issues labelled `good first issue` are a good start. For
  anything big, open an issue first so we can agree on the approach.

## Development setup

You need [Rust](https://rustup.rs) (stable). Nothing else: the database is
SQLite and everything is compiled into one program.

```bash
git clone https://github.com/dincertekin/bloogla
cd bloogla
cargo run
# open the setup link it prints (http://localhost:8080/setup?code=...)
```

Your local site lives in `data/` and `uploads/` (both ignored by git). Delete
them to start fresh.

Before sending a pull request, run what CI runs:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo test` runs the whole site the way a browser uses it (see
`src/tests/`), and also checks that every piece of interface text is
translated and that every bundled theme draws every page.

The visual editor (`admin/static/js/visual-editor.js`) is built from
`admin/visual-editor/` with Node.js; you only need that to change the editor
itself (see its [README](admin/visual-editor/README.md)).

## How the code is organised

The code is written to be read by people new to Rust: plain names, small
functions, and a comment saying *why* wherever it isn't obvious.

```
.
├── Cargo.toml              # Dependencies, grouped by what they're for
├── askama.toml             # Tells Askama where the admin templates are
├── Dockerfile              # Builds a small image with just the binary
├── admin/                  # The admin panel's look (compiled into the binary)
│   ├── templates/          #   HTML pages (Askama), one per screen
│   ├── static/             #   CSS and JS, served at /static
│   └── visual-editor/      #   Source of the visual editor (built into static/js/visual-editor.js)
├── themes/                # The bundled themes: default, story, gazette (written to themes/ on first start)
├── migrations/             # Database tables, applied automatically on start
└── src/
    ├── main.rs             # Start here: reads the command and runs it
    ├── app/                # Building blocks used everywhere
    │   ├── config.rs       #   BLOOGLA_* environment variables
    │   ├── state.rs        #   AppState: what every request handler can reach
    │   ├── models.rs       #   Shared data types: Post, Tag, Role, CurrentUser...
    │   ├── security.rs     #   Passwords, random codes, the security log
    │   ├── secrets.rs      #   Encrypting stored secrets (mail password, 2FA keys)
    │   └── totp.rs         #   Two-factor login codes
    ├── commands/           # The command line: serve, backup, reset-password, disable-2fa
    ├── server/             # Running the website
    │   ├── mod.rs          #   Starting the server, sessions, shutdown
    │   ├── routes.rs       #   Every URL and the handler that answers it
    │   ├── middleware.rs   #   CSRF, sign-in and role checks, ETags, logging...
    │   ├── assets.rs       #   Serving the embedded admin CSS/JS
    │   └── tls.rs          #   Built-in HTTPS
    ├── handlers/           # The code behind each URL
    │   ├── site/           #   Public website: pages, feeds, comments, analytics
    │   └── admin/          #   Admin panel: one file per screen (posts.rs, settings.rs...)
    ├── db/                 # Reading and writing the database
    │   ├── settings.rs     #   Every site setting and its default
    │   ├── posts.rs        #   Finding, saving and deleting posts and pages
    │   └── tags.rs, users.rs
    ├── content/            # Turning Markdown into web pages
    │   ├── markdown.rs     #   Markdown → safe HTML, excerpts
    │   ├── shortcodes.rs   #   [youtube], [toc] and theme shortcodes
    │   ├── seo.rs          #   Meta, Open Graph and JSON-LD tags
    │   └── text.rs         #   Slugs, escaping, dates
    ├── services/           # Work besides answering pages
    │   ├── email.rs        #   Sending email over SMTP (comment notifications)
    │   ├── backup.rs       #   Database backups and the downloadable .zip
    │   ├── updates.rs      #   Checking GitHub for new versions, installing them, restarting
    │   └── themes/         #   Loading and rendering themes, and their options
    ├── i18n/               # Interface languages: en.rs, tr.rs, and mod.rs with the shared code
    └── tests/              # Tests that use the whole site like a browser (see tests/mod.rs)
```

Created at runtime: `data/` (database, backups), `uploads/`.

## Adding something new

- **A new page or endpoint:** write a handler in `src/handlers/`, then add one `.route(...)` line in `src/server/routes.rs`.
- **A new admin screen:** add `admin/templates/<name>.html` and `src/handlers/admin/<name>.rs` (the template struct sits next to its handlers), then add the route in the right role group.
- **Something on an admin page that shows saved values** (a count, a name): give it an `id` and `data-live`. After any change on the page, admin.js fetches the page again and swaps in the new version, so nobody has to reload. A change that affects the whole page (like its language) answers with `saved_and_reload` instead.
- **A new setting:** add a field to `Settings` in `src/db/settings.rs` and give it a default there.
- **A database change:** add a new file to `migrations/` (never edit one that has already run).
- **A test:** copy one from `src/tests/`. `TestSite::with_owner()` gives you a fresh site with the owner signed in; `site.get(...)` and `site.post(...)` work like a browser.
- **New interface text:** wrap it in `me.t("...")` and add its Turkish translation to `src/i18n/tr.rs`; `cargo test` lists anything missing.
- **A new language:** copy `src/i18n/tr.rs` to `<code>.rs`, translate it, and add one line to `LANGUAGES` in `src/i18n/mod.rs`.

## House rules

- **Plain, friendly words** in everything visitors and admins see. No
  jargon, no exclamation-mark marketing.
- **Keep the look:** the admin panel reuses its existing components (cards,
  buttons, badges, tabs) and colours. No gradients or emoji in the interface.
  For bigger visual changes, open an issue with a screenshot or sketch first.
- **No inline scripts** in admin templates; the Content Security Policy blocks
  them, and a test checks. Behaviour lives in `admin/static/js/` and is switched
  on with `data-` attributes.
- **Every interface text is translated:** wrap it in `me.t("...")` and add the
  Turkish line in `src/i18n/tr.rs`.
- **Fewer, finished features** beat many half-done ones. Small pull requests
  with a test are the easiest to review.

## Releasing (maintainers)

Set `version` in `Cargo.toml`, then push a matching tag (`git tag v1.0.0 && git push --tags`). GitHub Actions (`.github/workflows/release.yml`) builds the program for Linux servers (x86_64 and ARM), signs it, creates a GitHub release with it and the list of changes (running sites pick that up as an available update), and publishes the Docker image as `ghcr.io/<owner>/bloogla:<version>` and `:latest`. Make the package public once in GitHub (Packages → bloogla → Package settings) so anyone can pull it.

**The release signing key.** Sites only install programs signed with it, so a hacked GitHub account alone can't push code onto them. The public half is `SIGNING_KEY` in `src/services/updates.rs`; the private half is a file you keep (`bloogla-release-signing-key.pem`). Once, add it to the repository: GitHub → Settings → Secrets and variables → Actions → New repository secret, named `BLOOGLA_SIGNING_KEY`, with the file's whole text as the value (or `gh secret set BLOOGLA_SIGNING_KEY < bloogla-release-signing-key.pem`). Keep a copy somewhere safe and never commit it. If it's ever lost or leaked, make a new one (`openssl genpkey -algorithm ed25519 -out bloogla-release-signing-key.pem`), put its public half in `SIGNING_KEY` (`openssl pkey -in bloogla-release-signing-key.pem -pubout -outform DER | tail -c 32 | xxd -p -c 64`), and release that version by hand: sites can only install versions signed with the key they already have.
