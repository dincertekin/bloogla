# AGENTS.md

Guidance for AI coding agents working on Bloogla. Humans: see
[CONTRIBUTING.md](CONTRIBUTING.md), which covers the same ground.

## What Bloogla is

A single-binary blog engine and CMS in Rust (axum + SQLite), a lighter
WordPress alternative for **non-developers**: one program runs the site, the
admin panel, migrations and HTTPS. Two things follow from that audience:

- Behaviour and wording must be polished and plain. No jargon in the UI.
- The maintainer is learning Rust: keep code simple, explicit and commented
  ("why", not "what"). Prefer a few finished features over clever abstractions.

## Commands

```bash
cargo run                                   # dev server on :8080, prints a setup link
cargo test                                  # all tests (fast; full-request tests included)
cargo fmt                                   # format
cargo clippy --all-targets -- -D warnings   # lint; CI fails on any warning
```

Run all three checks before saying a change is done. The visual editor bundle
(`admin/static/js/visual-editor.js`) is built from `admin/visual-editor/`
with `npm install && npm run build`; don't edit the built file by hand.

## Where things are

- `src/main.rs` → `src/commands/` (CLI) → `src/server/` (startup, `routes.rs` = every URL, `middleware.rs`)
- `src/handlers/admin/<screen>.rs` + `admin/templates/<screen>.html` (Askama, compiled in)
- `src/handlers/site/` public pages, rendered by themes (Tera, loaded at runtime from `themes/`)
- `src/db/` queries; `src/db/settings.rs` is the typed `Settings` struct (all settings + defaults)
- `src/services/` email, backups, themes, updates (GitHub check, signed self-update, restart)
- `src/i18n/` languages: `tr.rs` maps English text → Turkish; `mod.rs` has `THEME_STRINGS`
- `src/tests/` full-request tests with `TestSite` (fresh DB per test)
- `admin/static/js/admin.js` shared admin behaviour, documented at the top of the file
- `themes/{default,story,gazette}` bundled themes, embedded and installed on start
- `migrations/` SQL migrations, applied on start
- `docs/` user and theme-author documentation

## Rules that tests or CI enforce

- **Every interface string is translated.** Admin templates use
  `me.t("English text")` / `me.tv(...)` / `me.count(...)`; add the Turkish line
  to `src/i18n/tr.rs`. `cargo test` lists missing ones. Theme text goes through
  `THEME_STRINGS` ids (`{{ t.back_to_all_posts }}`), because Tera can't look up
  keys containing dots.
- **No inline scripts or `on*=` handlers in admin templates.** The admin CSP
  forbids them and a test checks. Add behaviour to `admin/static/js/` and
  switch it on with `data-` attributes.
- **Bundled themes must render every page.** `src/tests/themes.rs` opens each
  page type in each theme.
- **Formatting and clippy are clean** (`-D warnings`).

## Conventions

- **Admin interactions use htmx.** Forms post with `hx-post` and get back an
  `alert(lang, kind, message)` HTML snippet (`src/handlers/admin/mod.rs`).
  - Parts of a page that show saved values get an `id` and `data-live`;
    admin.js refetches them after any change, so nothing needs a reload.
  - Changes that affect the whole page (its language) answer with
    `saved_and_reload(lang)`.
  - Validation failures may return 422 with HTML; admin.js swaps those too.
- **Settings:** add a field and default to `Settings` in `src/db/settings.rs`,
  save with `settings::save`. Settings are cached in memory (`settings::load`).
- **Database changes:** add a new migration file; never edit one that has run.
- **Bundled themes:** after changing a bundled theme's files, bump `version` in
  its `theme.toml`, or existing sites keep their old copy.
- **Tera pitfalls:** `not x` or `x == y` on an undefined variable is an error;
  use `x is defined`. Variables set with `{% set %}` inside a `for` loop stay
  in that loop.
- **Security:** state-changing requests are CSRF-checked through
  `Sec-Fetch-Site`/`Origin` (tests send `sec-fetch-site: same-origin`).
  Escape anything user-provided that goes into HTML built in Rust
  (`content::text::escape_html`).
- **UI design:** reuse the existing admin components (cards, `.btn`, badges,
  tabs, modals) and CSS variables. Monochrome, no gradients or emoji, short
  human copy. Propose visible design changes before making them.
- **Motion:** keep animations subtle and respect `prefers-reduced-motion`.
- **The terminal window:** what the person running Bloogla sees goes through
  `app::console` (start screen, `activity_t` news lines, `failed`), translated
  like the admin. In Docker/systemd it falls back to log lines by itself;
  don't print with `println!` from the server.

## Don't

- Don't commit, push or tag unless asked; the maintainer handles git.
- Don't commit secrets: `data/` (database, `secret.key`), `uploads/` and the
  release signing key (`bloogla-release-signing-key.pem`) stay out of git.
- Don't add new dependencies without a good reason; say why in `Cargo.toml`
  (dependencies are grouped and commented by purpose).
- Don't bring back features removed for v1 without asking: webmentions,
  newsletter, JSON API, custom fields, theme upload, WordPress import,
  categories (tags only), other site types.
