# AGENTS.md

The rules for working on Bloogla, for AI coding agents and people alike.
[CONTRIBUTING.md](CONTRIBUTING.md) has setup, a code tour and releasing. Keep
rules here, not there, so they never drift apart.

## What Bloogla is

A single-binary blog engine and CMS in Rust (axum + SQLite): a lighter
WordPress for **non-developers**. One program runs the site, the admin,
migrations and HTTPS. It runs on other people's servers and updates itself,
so a mistake can cost someone their site.

- Wording and behaviour are polished and plain. No jargon, no
  exclamation-mark marketing.
- The maintainer is learning Rust: keep code simple and explicit.
- When goals conflict: **performance, then security, then looks.** Never
  trade away anyone's data.

## How to work

- **One way to do each thing.** Owners do everything in the admin. The
  command line is only for getting back in (forgotten password, lost
  two-factor phone); `BLOOGLA_*` variables only for what's needed before the
  admin exists (address, port, HTTPS domain). Adding a way to do something
  means removing the old one. In your own work, pick one approach. Ask only
  when the decision is the maintainer's (product, scope, visible design),
  and recommend one option.
- **No overengineering.** Build what was asked, the simplest correct way. No
  speculative options or "for later" code; no new layer or helper unless it
  removes duplication that exists now. Tests, translations and docs are part
  of the feature. Fewer, finished features beat many half-done ones.
- **Follow the structure.** New code goes where "Where things are" says, in
  the shape of the nearest similar screen or service. Split a file when it
  does more than one screen or job.
- **Use the product's words:** post (never article), page, tag, person,
  theme, backup. Full names (`published_posts`, not `pp`), the same word for
  the same thing everywhere.
- **Comments are for a beginner:** short and friendly. Explain why, or a Rust
  detail that isn't obvious; never repeat the line. Each file starts with a
  `//!` line saying what it's for.

## Commands

```bash
cargo run                                   # dev server on :8080, prints a setup link
cargo test                                  # all tests, including full requests
cargo fmt
cargo clippy --all-targets -- -D warnings   # CI fails on any warning
```

The visual editor (`admin/static/js/visual-editor.js`) is built from
`admin/visual-editor/` with `npm install && npm run build`. Never edit the
built file.

## Where things are

- `src/main.rs` → `src/commands/` (CLI) → `src/server/` (startup;
  `routes.rs` = every address; `middleware.rs`)
- `src/handlers/admin/<screen>.rs` + `admin/templates/<screen>.html` (Askama,
  compiled in). New template structs go in `templates_answer_requests!` in
  `src/handlers/admin/mod.rs`.
- `src/handlers/site/`: public pages, drawn by themes (Tera, loaded from
  `themes/` at runtime)
- `src/db/`: every SQL query, one file per table. Handlers and services call
  these; only backup/restore touch whole database files.
  `src/db/settings.rs` is the typed `Settings` struct with every default.
- `src/services/`: email, backups, images, videos, import/export, themes,
  updates (GitHub check, signed self-update, restart)
- `src/i18n/`: `de.rs`, `fr.rs`, `lt.rs`, `tr.rs` map English → each
  language; `mod.rs` has `THEME_STRINGS`
- `src/tests/`: full-request tests with `TestSite` (fresh database each)
- `admin/static/js/admin.js`: shared admin behaviour, documented at the top
- `themes/`: the seven bundled themes, embedded and installed on start
- `migrations/`: SQL migrations, applied on start
- `docs/`: documentation for site owners and theme authors

## Done means

1. `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.
2. New behaviour has a full-request test in `src/tests/`; a bug fix has a
   test that fails without it.
3. You ran the program and tried the change (`cargo run`, or a release build
   in an empty folder for start-up, file or update changes). Some mistakes
   only show when the server starts.
4. `README.md` and `docs/` match: new settings, `BLOOGLA_*` variables,
   commands and theme variables are documented.
5. Visible changes are described in one plain sentence for the release notes.

Say plainly what you couldn't check (no browser, no real domain, no Linux).

## Checked by tests and CI

- **Every interface text is translated.** Templates use `me.t("...")`,
  `me.tv(...)`, `me.count(...)`; add a line to each file in `src/i18n/`.
  `cargo test` lists what's missing. Every language uses the polite "you"
  (Sie, vous, siz, jūs): warm, never informal. Theme text uses
  `THEME_STRINGS` ids (`{{ t.back_to_all_posts }}`), because Tera can't look
  up keys with dots.
- **No inline scripts or `on*=` handlers in admin templates** (the CSP
  forbids them). Put behaviour in `admin/static/js/`, switched on with
  `data-` attributes.
- **Every bundled theme draws every page** (`src/tests/themes.rs`).
- **The release tag matches `Cargo.toml`'s `version`.**

## Never break a running site

People update with one click and can't fix things by hand.

- **Saved data still reads:** the database, `data/secret.key`, encrypted
  values, sessions and backup `.zip` files from earlier versions. When
  changing code that reads stored data, test a value the previous version
  wrote (see `src/app/secrets.rs`).
- **Database changes only add.** New migration files, never edited old ones;
  add columns rather than rename or drop. Migrate data before it goes away.
- **Theme variables are a public API** (`post.*`, `t.*`, `theme_options`...).
  Never rename or remove one: add the new name, keep the old one, note it in
  `docs/themes.md`.
- **Addresses keep working** (`/post/...`, `/tag/...`, `/rss.xml`, page
  slugs, old redirects). Add a redirect whenever something moves.
- **Ask first** before changing the update-signing key or what's signed
  (`src/services/updates.rs`), `BLOOGLA_*` names, the backup format or the
  layout of Bloogla's folder.
- **Every platform we ship runs it:** Linux (x86_64, ARM) and Windows; macOS
  is only built from source. Unix-only code needs `#[cfg(unix)]` and a
  fallback.

## Performance and privacy

- **Public pages load nothing from other servers:** no CDNs, web fonts,
  analytics or trackers. (Embeds a writer adds are their choice.)
- **Keep pages light:** no query per item in a list, no new JavaScript on
  public pages without a clear reason, images lazy-loaded with sizes, except
  the first big picture (`fetchpriority="high" decoding="sync"`). The
  bundled themes score 100 on Lighthouse; keep it.
- **The server only goes online for what the owner turned on** (update
  checks, email, HTTPS certificates). Anything new that calls out is opt-in
  and explained where it's switched on.
- **Analytics stay anonymous:** no cookies, no stored IP addresses.
- **Keep the binary small.** Check `cargo tree` after adding or upgrading a
  dependency; turn off unused default features.

## Security and safety

- **CSRF:** changes are checked with `Sec-Fetch-Site`/`Origin` (tests send
  `sec-fetch-site: same-origin`).
- **Escape** anything user-provided in HTML built in Rust
  (`content::text::escape_html`).
- **Check permissions on the server**, not by hiding buttons: routes sit in
  role groups in `src/server/routes.rs`.
- **Fetching an address from a user or an import:** refuse local and private
  addresses (also after redirects), limit size, time and redirects, accept
  only the expected file types.
- **Uploads and imports are hostile until checked:** limit size, check the
  real type, never trust names or paths inside files (no `..`).
- **Never panic on input:** no `unwrap()`/`expect()` on anything a request or
  file controls. A broken file gets a friendly error.
- **Destructive actions** (delete, restore, empty the trash, import over
  content) ask in plain words first; the big ones make a backup first.
- **Safe defaults:** new settings start in the safer, quieter position.
- **Log security events** (sign-ins, password and role changes) with
  `app::security::log_event`. Never log passwords, tokens or codes.

## Conventions

- **Admin interactions use htmx.** Forms post with `hx-post` and get back an
  `alert(lang, kind, message)` snippet (`src/handlers/admin/mod.rs`).
  Validation failures may return 422 with HTML.
  - Parts of a page showing saved values get an `id` and `data-live`;
    admin.js refetches them after any change.
  - Changes to the whole page (its language) answer `saved_and_reload(lang)`.
- **Settings:** a field and default in `Settings` (`src/db/settings.rs`),
  saved with `settings::save`, cached in memory (`settings::load`).
- **Bundled themes:** after changing one, raise `version` in its
  `theme.toml`, or sites keep their old copy.
- **Tera pitfalls:** `not x` or `x == y` on an undefined variable is an
  error, so use `x is defined`. `{% set %}` inside a `for` loop stays in it.
- **Messages** say what happened and what to do next ("The file is not a
  WordPress export. In WordPress, use Tools → Export."), never a raw error.
- **Design:** reuse the admin's components (cards, `.btn`, badges, tabs,
  modals) and CSS variables. Monochrome, no gradients or emoji, short human
  copy. Propose visible design changes before making them (people: open an
  issue with a sketch first).
- **Accessibility:** every field has a label, everything works with the
  keyboard, pop-ups keep focus inside and give it back, images have alt
  text, contrast is enough in light and dark.
- **Motion:** never delays, hides or dims content someone is waiting for.
  Themes don't animate page changes. In the admin, motion is part of the
  premium feel: on a page change the frame stays still, the old content
  fades out quickly and the new one rises in ("MOTION" in
  `admin/static/css/style.css`). Never show two pages at once, never scale
  text. Motion means the same everywhere: a shake is "no" (only for what
  someone typed), a check drawing itself is "done", rows close up when
  removed, pop-ups rise in. Reuse `shake` and `showDone` (`admin.js`).
  Always respect `prefers-reduced-motion`.
- **The terminal window:** what the person running Bloogla sees goes through
  `app::console` (`activity_t`, `failed`), translated like the admin. Never
  `println!` from the server.

## Dependencies

- Use what's in `Cargo.toml`. If something new is really needed, ask first
  and say why; then add a comment in `Cargo.toml` saying what it's for.
- When upgrading, read the changelog for changes that only show at run time
  (axum 0.8 changed route syntax: it compiled, then crashed on start), and
  run the program.
- Stay on Tera 1: Tera 2 would break themes.

## Releases and git

- `1.0.x` for fixes and dependency upkeep, `1.x.0` for features; never a
  change that needs people to edit files by hand.
- Set `version` in `Cargo.toml`; the maintainer tags and publishes.
- Credit translators of new languages in the release notes.
- Commit messages: one plain sentence about what changed for the person
  using Bloogla ("Faster pages on phones"). Release commits start with
  `Release 1.x.y:`. No `feat:`/`fix:` prefixes. Never rewrite pushed history.
- Don't commit, push or tag unless asked: the maintainer handles git.
- Never commit secrets: `data/`, `uploads/` and
  `bloogla-release-signing-key.pem` stay out of git.
