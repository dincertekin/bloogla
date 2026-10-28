# Contributing to Bloogla

Thanks for helping! Bloogla is for people who aren't developers, so every
change answers one question: **does it make Bloogla simpler, faster or safer
for them?**

The rules (how to work, what "done" means, what must never break) are in
[AGENTS.md](AGENTS.md). It's written so AI coding tools follow them too, but
it's the rulebook for everyone. Please read it before your first pull
request.

## Ways to help

- **Report a bug or an idea:** [open an issue](https://github.com/dincertekin/bloogla/issues/new/choose).
  Security problems go [privately](SECURITY.md).
- **Translate:** no programming needed. See [docs/translating.md](docs/translating.md).
- **Make a theme:** HTML and CSS, no Rust. See [docs/themes.md](docs/themes.md).
- **Write code:** issues labelled `good first issue` are a good start. For
  something big, open an issue first.

## Setting up

You need [Rust](https://rustup.rs). Nothing else.

```bash
git clone https://github.com/dincertekin/bloogla
cd bloogla
cargo run    # then open the setup link it prints
```

Your local site is in `data/` and `uploads/`. Delete them to start fresh.

Before a pull request, run what CI runs:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test   # uses the whole site like a browser, checks translations and themes
```

The visual editor is built from `admin/visual-editor/` with Node.js. You
only need that to change the editor itself
([how](admin/visual-editor/README.md)).

## Where things are

```
admin/templates/     the admin's HTML, one file per screen
admin/static/        the admin's CSS and JavaScript
themes/              the seven bundled themes
migrations/          database tables, applied on start
src/main.rs          start here
src/server/          routes.rs (every address), middleware.rs (checks on every request)
src/handlers/admin/  the code behind each admin screen
src/handlers/site/   the public pages
src/db/              every database query, one file per table
src/services/        email, backups, images, videos, import, export, themes, updates
src/content/         Markdown, shortcodes, SEO tags, dates
src/i18n/            translations
src/tests/           tests that use the site like a browser
```

### Follow one click

Here is **Settings → Redirects → Add Redirect**, start to finish. Every admin
screen works the same way.

1. `admin/templates/settings.html`: the form has `hx-post="/admin/redirects"`.
   [htmx](https://htmx.org) sends it and puts the answer in the page.
2. `src/server/routes.rs`: `.route("/admin/redirects", post(admin::redirects::create))`,
   in the `admins` group, so only admins get through.
3. `src/server/middleware.rs`: checks the request came from this site and who
   is signed in.
4. `src/handlers/admin/redirects.rs`: `create` gets `State(state)` (the
   database), `Extension(me)` (who's asking) and `Form(form)` (what was
   typed), and checks the input.
5. `src/db/redirects.rs`: `redirects::add` saves it. All SQL is in `src/db/`.
6. `admin/templates/redirect_item.html`: the new row, which htmx adds to the
   list.

Public pages are the same until step 6, where a theme draws the page.

Three Rust patterns you'll see everywhere:

- `let Some(x) = ... else { return ... };`: stop here if there's nothing.
- `?`: stop here and pass the error on.
- `.await`: wait for the database or network without blocking other visitors.

## Adding something new

- **A page:** a handler in `src/handlers/`, and one `.route(...)` in `src/server/routes.rs`.
- **An admin screen:** `admin/templates/<name>.html` and
  `src/handlers/admin/<name>.rs`. Add the template to `templates_answer_requests!`
  in `src/handlers/admin/mod.rs`, and the route to the right role group.
- **A setting:** a field and its default in `src/db/settings.rs`.
- **A query:** a plainly named function in `src/db/`, called from the handler.
- **A table or column:** a new file in `migrations/`. Never edit an old one.
- **Interface text:** `me.t("...")`, and a line in each file in `src/i18n/`.
  `cargo test` lists what's missing.
- **A test:** copy one from `src/tests/`. `TestSite::with_owner()` is a fresh
  site with the owner signed in.

## Releasing (maintainers)

1. **Check new screens like a visitor:** Lighthouse in mobile mode on each
   theme's home, post, tag, page and archive (keep 100 for performance and
   accessibility), and [axe](https://github.com/dequelabs/axe-core) (WCAG 2.2
   AA) on those pages and the admin, light and dark.
2. **Set `version`** in `Cargo.toml`: `1.0.x` for fixes, `1.x.0` for features.
3. **Tag and push:** `git tag v1.3.0 && git push --tags`.

GitHub Actions then builds the Linux and Windows programs, signs them,
publishes the release (running sites see it in Settings → Updates) and the
Docker image `ghcr.io/dincertekin/bloogla`.

**The signing key.** Sites only install programs signed with it. The public
half is `SIGNING_KEY` in `src/services/updates.rs`. The private half,
`bloogla-release-signing-key.pem`, is the repository secret
`BLOOGLA_SIGNING_KEY` (`gh secret set BLOOGLA_SIGNING_KEY < bloogla-release-signing-key.pem`).
Keep a copy safe and never commit it. If it's lost or leaked: make a new one
(`openssl genpkey -algorithm ed25519 -out bloogla-release-signing-key.pem`),
put its public half in `SIGNING_KEY`
(`openssl pkey -in bloogla-release-signing-key.pem -pubout -outform DER | tail -c 32 | xxd -p -c 64`),
and tell people to install that version by hand: sites only accept the key
they already have.
