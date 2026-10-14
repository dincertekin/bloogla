# AGENTS.md

Guidance for AI coding agents working on Bloogla. Humans: see
[CONTRIBUTING.md](CONTRIBUTING.md), which covers the same ground.

## What Bloogla is

A single-binary blog engine and CMS in Rust (axum + SQLite), a lighter
WordPress alternative for **non-developers**: one program runs the site, the
admin panel, migrations and HTTPS. It runs on other people's servers and
updates itself, so a mistake can cost someone their site, not just a red test.

- Behaviour and wording must be polished and plain. No jargon in the UI.
- The maintainer is learning Rust: keep code simple and explicit.
- When goals pull apart: **performance, then security, then looks**. Never
  trade away a person's data for any of them.

## How to work

- **One way to do each thing.** In the product: every task has exactly one
  way to do it. Owners do things in the admin panel. The command line is
  only for getting back in when the admin can't be reached (a forgotten
  password, a lost two-factor phone), and `BLOOGLA_*` variables only for what
  is needed before the admin exists (address, port, HTTPS domain). If you
  add a way to do something, remove the old one. In your work: pick one
  approach and do it; ask only when the decision is the maintainer's
  (product, scope, visible design), and then recommend one option.
- **No overengineering.** Build what was asked, the simplest way that is
  correct. No speculative options, settings or "for later" code; no new
  layer or helper unless it removes duplication that exists now. The
  finished feature still includes its test, translations and docs: those
  aren't extras (see "Done means").
- **Follow the structure.** New code goes where "Where things are" says;
  copy the shape of the nearest similar screen or service. Add a new folder
  or kind of file only for a clear reason. Split a file when it starts doing
  more than one screen or job.
- **Names use the product's words:** post (never article), page, tag,
  person, theme, backup. Clear and complete (`published_posts`, not `pp`),
  and the same word for the same thing everywhere.
- **Comments are for a beginner:** friendly, short and direct. Explain why
  the code does something, or a Rust detail that isn't obvious; don't
  repeat what the line says. Each file starts with a `//!` line saying what
  it's for.

## Commands

```bash
cargo run                                   # dev server on :8080, prints a setup link
cargo test                                  # all tests (fast; full-request tests included)
cargo fmt                                   # format
cargo clippy --all-targets -- -D warnings   # lint; CI fails on any warning
```

The visual editor bundle (`admin/static/js/visual-editor.js`) is built from
`admin/visual-editor/` with `npm install && npm run build`; don't edit the
built file by hand.

## Where things are

- `src/main.rs` → `src/commands/` (CLI) → `src/server/` (startup, `routes.rs` = every URL, `middleware.rs`)
- `src/handlers/admin/<screen>.rs` + `admin/templates/<screen>.html` (Askama, compiled in);
  a new template struct goes in the `templates_answer_requests!` list in
  `src/handlers/admin/mod.rs` so handlers can return it
- `src/handlers/site/` public pages, rendered by themes (Tera, loaded at runtime from `themes/`)
- `src/db/` queries; `src/db/settings.rs` is the typed `Settings` struct (all settings + defaults)
- `src/services/` email, backups, themes, updates (GitHub check, signed self-update, restart)
- `src/i18n/` languages: `de.rs`, `fr.rs`, `lt.rs` and `tr.rs` map English text → German, French, Lithuanian and Turkish; `mod.rs` has `THEME_STRINGS`
- `src/tests/` full-request tests with `TestSite` (fresh DB per test)
- `admin/static/js/admin.js` shared admin behaviour, documented at the top of the file
- `themes/{default,story,gazette}` bundled themes, embedded and installed on start
- `migrations/` SQL migrations, applied on start
- `docs/` user and theme-author documentation

## Done means

A change is done when all of these hold, not before:

1. `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.
2. New behaviour has a full-request test in `src/tests/`; a bug fix has a
   test that fails without the fix.
3. You ran the program and tried the change (`cargo run`, or a release build
   in an empty folder for start-up, file or update changes). Compiling isn't
   enough: some mistakes only show when the server starts.
4. `README.md` and `docs/` match: new settings, `BLOOGLA_*` variables, CLI
   commands and theme variables are documented (`docs/running.md`,
   `docs/themes.md`).
5. Visible changes are described in one plain sentence for the release notes.

Say plainly what you could not check (no browser, no real domain, no Linux).

## Rules that tests or CI enforce

- **Every interface string is translated.** Admin templates use
  `me.t("English text")` / `me.tv(...)` / `me.count(...)`; add a line to each
  language file in `src/i18n/` (`de.rs`, `fr.rs`, `lt.rs`, `tr.rs`). `cargo test` lists missing ones. Every
  language uses the polite, formal "you" (Sie, vous, siz, jūs), warm but never
  informal. Theme text goes through
  `THEME_STRINGS` ids (`{{ t.back_to_all_posts }}`), because Tera can't look up
  keys containing dots.
- **No inline scripts or `on*=` handlers in admin templates.** The admin CSP
  forbids them and a test checks. Add behaviour to `admin/static/js/` and
  switch it on with `data-` attributes.
- **Bundled themes must render every page.** `src/tests/themes.rs` opens each
  page type in each theme.
- **Formatting and clippy are clean** (`-D warnings`).
- **Release tag matches `Cargo.toml`'s `version`**, or the release workflow stops.

## Never break a running site

People update with one click and can't fix things by hand. After any update:

- **Saved data still reads.** The database, `data/secret.key`, encrypted
  values, sessions and backup `.zip` files from earlier versions must keep
  working. When upgrading a library or changing code that reads stored data,
  add a test that reads a value written by the previous version (see the
  test in `src/app/secrets.rs`).
- **Database changes only add.** New migration files, never edited old ones;
  prefer adding columns over renaming or dropping. Data that goes away is
  migrated first.
- **Themes keep working.** The variables a theme gets (`post.*`, `t.*`,
  `theme_options`, ...) are a public API for other people's themes: don't
  rename or remove one; add the new name, keep the old one working and note
  it in `docs/themes.md`.
- **Addresses keep working.** `/post/...`, `/tag/...`, `/rss.xml`, page slugs
  and old addresses that redirect: changing them loses visitors and search
  ranking. Add a redirect whenever something moves.
- **Ask first before changing** the update-signing key or what is signed
  (`src/services/updates.rs`), `BLOOGLA_*` variable names, the backup format
  or the layout of the folder Bloogla runs in.
- **Every platform we ship runs it:** Linux (x86_64, ARM) and Windows; macOS
  is only built from source (for development). Unix-only code needs
  `#[cfg(unix)]` and a working fallback.

## Performance and privacy

- **Public pages load nothing from other servers.** No CDNs, web fonts,
  analytics or tracking scripts; fonts and assets ship inside Bloogla. (Embeds
  a writer adds to a post are their choice.)
- **Keep pages light.** No database query per item in a list, no new
  JavaScript on public pages without a clear reason, images lazy-loaded with
  sizes. The bundled themes score 100 on Lighthouse; keep it that way.
- **The server reaches the internet only for things the owner turned on**
  (update checks, sending email, HTTPS certificates). Anything new that calls
  out is opt-in and explained where it's switched on.
- **Analytics stay anonymous:** no cookies, no stored IP addresses.
- **Keep the binary small.** Check `cargo tree` after adding or upgrading a
  dependency; turn off default features that pull in things we don't use.

## Security and safety

- **CSRF:** state-changing requests are checked through
  `Sec-Fetch-Site`/`Origin` (tests send `sec-fetch-site: same-origin`).
- **Escape** anything user-provided that goes into HTML built in Rust
  (`content::text::escape_html`).
- **Check permissions on the server**, not by hiding buttons: routes sit in
  role groups in `src/server/routes.rs`.
- **Fetching an address from a user or an imported file** (images in an
  import, links): refuse local and private network addresses (also after
  redirects), limit size, time and redirects, and accept only the expected
  file types.
- **Uploaded and imported files are hostile until checked:** limit size, check
  the real type, never trust names or paths inside archives (no `..`).
- **Never panic on input.** No `unwrap()`/`expect()` on anything a request,
  upload or imported file controls; a broken file gets a friendly error.
- **Destructive actions** (delete, restore, emptying the trash, importing over
  existing content) ask in plain words first, and the big ones make an
  automatic backup before they start.
- **Safe defaults:** new settings start in the safer, quieter position.
- **Log security events** (sign-ins, password and role changes) with
  `app::security::log_event`; never log passwords, tokens or codes.

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
- **Bundled themes:** after changing a bundled theme's files, bump `version` in
  its `theme.toml`, or existing sites keep their old copy.
- **Tera pitfalls:** `not x` or `x == y` on an undefined variable is an error;
  use `x is defined`. Variables set with `{% set %}` inside a `for` loop stay
  in that loop.
- **Messages:** say what happened and what to do next ("The file is not a
  WordPress export. In WordPress, use Tools → Export."), never a raw error.
- **UI design:** reuse the existing admin components (cards, `.btn`, badges,
  tabs, modals) and CSS variables. Monochrome, no gradients or emoji, short
  human copy. Propose visible design changes before making them.
- **Accessibility:** every field has a label, everything works with the
  keyboard, pop-ups keep focus inside and give it back, images have alt text,
  text keeps enough contrast in light and dark mode.
- **Motion:** smooth, premium-feeling motion is part of Bloogla: page changes,
  tabs, new items and hovers should glide, never jump. Moving between pages,
  the frame (header, sidebar) stays still and only the content changes: the
  old content fades out quickly, then the new one rises in (see "MOTION" in
  `admin/static/css/style.css`). Never let two pages show at once (a plain
  crossfade looks blurry) and don't scale text. Motion means something, the
  same everywhere: a shake is "no" (only for what someone typed), a check
  drawing itself is "done", rows close up when removed, pop-ups rise in.
  Reuse these (`shake`, `showDone` in `admin.js`) rather than inventing new
  ones. Always respect `prefers-reduced-motion`.
- **The terminal window:** what the person running Bloogla sees goes through
  `app::console` (start screen, `activity_t` news lines, `failed`), translated
  like the admin. In Docker/systemd it falls back to log lines by itself;
  don't print with `println!` from the server.

## Dependencies

- Use what is already in `Cargo.toml`. If something new is really needed,
  ask first and say why; once agreed, add a comment in `Cargo.toml` saying
  what it's for (dependencies are grouped by purpose).
- When upgrading, read the changelog for changes that only show at run time
  (axum 0.8 changed route syntax: the build passed, the server crashed on
  start). Then run the program, not just the tests.
- Stay on Tera 1: Tera 2 changes the template language and would break themes.

## Releases

- Versions: `1.0.x` fixes and dependency upkeep only; `1.x.0` new features;
  never a change that needs people to edit files by hand.
- Set `version` in `Cargo.toml`; the maintainer tags and publishes
  (CONTRIBUTING.md → Releasing).
- Credit translators of new languages in the release notes.

## Don't

- Don't commit, push or tag unless asked; the maintainer handles git.
- Don't commit secrets: `data/` (database, `secret.key`), `uploads/` and the
  release signing key (`bloogla-release-signing-key.pem`) stay out of git.
