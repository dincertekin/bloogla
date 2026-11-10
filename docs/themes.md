# Writing themes

A theme is a folder in `themes/`: a `theme.toml`, HTML templates in
`templates/` ([Tera](https://keats.github.io/tera/docs/)) and files in
`static/`. No Rust needed.

## Start in four steps

1. Copy `themes/default` to `themes/my-theme`.
2. Change the name in `theme.toml`.
3. Restart Bloogla and pick your theme in **Admin → Appearance**.
4. Edit the templates and CSS. While Bloogla runs with `cargo run`, changes
   show when you reload the page.

```toml
name = "My Theme"
version = "1.0.0"
author = "Your Name"
description = "One sentence about it."
preview_image = "static/preview.png"   # optional, shown in Appearance
```

A theme with a mistake is never used: Appearance shows what's wrong, and the
site keeps the default theme.

The seven bundled themes are good starting points: Default (a blog with
cards), Story (calm reading), Gazette (a newspaper), Folio (photographs),
Notebook (one quiet column), Atelier (warm, rounded) and Brief (a business's
news).

## Templates

| Template       | Shows                                   | Required |
| -------------- | --------------------------------------- | -------- |
| `index.html`   | The home page and search results        | Yes      |
| `post.html`    | One post                                | Yes      |
| `tag.html`     | Posts with a tag                        | Yes      |
| `page.html`    | A page (else `post.html`)               | No       |
| `archive.html` | Every post by year, at `/archive` (else `page.html`) | No |
| `404.html`     | "Page not found"                        | No       |

Refer to other templates by their path in the theme:
`{% extends "templates/layout.html" %}`. Link to your own files with
`theme_url` and the version, so browsers keep them until the theme changes:

```html
<link rel="stylesheet" href="{{ theme_url }}/static/css/style.css?v={{ asset_version }}">
```

## What templates get

**Every template:**

| Variable | What it is |
| -------- | ---------- |
| `blog_name`, `blog_description`, `base_url` | The site's name, description and address |
| `lang` | The site's language code, like `tr` |
| `t` | Interface text in the site's language: `{{ t.back_to_all_posts }}` |
| `menu` | The menu: a list with `label` and `url` |
| `blog_url` | Where the posts are: `/`, or `/blog` when a page is the home page. Link "all posts" here |
| `tags` | Tags with public posts |
| `search_query` | What was searched for |
| `show_views` | Whether to show view counts |
| `seo_head` | Put `{{ seo_head \| safe }}` in `<head>`: SEO tags, verification tags, a `generator` tag naming Bloogla (without its version), and the formula files on pages with formulas |
| `scheme_script` | Put `{{ scheme_script \| safe }}` in `<head>` (see [Light and dark](#light-and-dark)) |
| `theme_url`, `asset_version` | Your theme's address and version |
| `theme_options` | The [options](#theme-options) the owner chose |

**Lists** (home, tag, search): `posts` (each with `post.url`; search
results include pages) and `pagination` (`current`, `total_pages`,
`prev_url`, `next_url`).

**A post or page:** `post` and `content_html` (the text, as HTML). `post`
includes `title`, `url`, `created_at` (the date, ready to show),
`reading_time`, `tags`, `views`, `author_name` and the cover: `cover_image`,
`cover_width`, `cover_height`, and `cover_small` and `cover_thumb` (800 and
400 pixels wide, for `srcset`).

**Posts only:** `previous_post` and `next_post` (each with `title` and `url`,
empty at the ends), and with comments on: `comments_enabled`, `comments`,
`comment_action` and `comment_notice` (see `comments.html` in
[Shared parts](#shared-parts)).

**Email subscriptions:** put `{{ subscribe_form | safe }}` at the end of a
post, before the comments. It's the "Get new posts by email" form when the
owner turned subscriptions on (Settings → Subscribers), and empty otherwise
(and on pages). It uses the comment form's classes plus `contact-form` and
`subscribe-form`, so it looks like your contact form. Added in Bloogla 1.4.

**The archive:** `archive`, a list of years (newest first), each with `year`
and `posts` (each with `title`, `url` and `date`, like `Oct 2`). Without
`archive.html`, it's shown with `page.html`, the list in `content_html`.

**Inside `content_html`,** style these too:

- uploaded videos and `[video]`: `<video class="media">`;
- the contact form: the comment form's classes (`comment-form`,
  `comment-fields`, `comment-trap`, `comment-notice`) plus `contact-form`;
- formulas: `<span data-math-style="inline">` or `"display"`.

A draft opened with a private preview link looks like a normal post, without
comments.

## Theme options

Settings the owner changes in **Appearance**. List them in `theme.toml`;
templates read `theme_options.<name>`:

```toml
[[options]]
name = "accent_color"
label = "Accent color"
type = "color"   # text, textarea, color, checkbox, select (with choices = [...]) or image
default = "#2563eb"
hint = "Used for links, buttons and highlights."
```

```html
<style>:root { --accent: {{ theme_options.accent_color }}; }</style>
```

Values are checked before they're saved (colors look like `#2563eb`, images
come from Media or `https://`, a select only takes its choices), so they're
safe in styles.

Every bundled theme has the same six, so switching themes keeps the same
choices: `logo`, `color_scheme` ("Follow the visitor's device", "Always
light", "Always dark"), `scheme_button` (on at first), accent color, footer
text and "Powered by Bloogla". A new option in a bundled theme needs its
label, hint and choices in each language file (`cargo test` lists missing
ones).

### Light and dark

When the owner picks "Always light" or "Always dark", `<html>` gets
`data-scheme="light"` or `"dark"`. Write your dark colors for both cases:

```css
@media (prefers-color-scheme: dark) {
    :root:not([data-scheme="light"]) { --bg: #121214; }
}
:root[data-scheme="dark"] { --bg: #121214; }
```

To let visitors switch: include `templates/colors.html` and
`templates/scheme_button.html` (both [shared](#shared-parts), so you don't
need copies), and put `{{ scheme_script | safe }}` in `<head>`. It
applies the visitor's choice before the page is drawn. The choice stays in
their browser, never in a cookie.

## Rules

- **Speed first.** Don't animate page changes. The first big picture uses
  `fetchpriority="high" decoding="sync"`; pictures further down use
  `loading="lazy" decoding="async"`.
- **No inline scripts.** Pages have a security policy: scripts, styles and
  fonts come from the site; images and media from any HTTPS address; embeds
  from YouTube and Vimeo. Put JavaScript in `static/js/`. If you need more,
  list it in `theme.toml`:

  ```toml
  [csp]
  font-src = ["https://fonts.gstatic.com"]
  ```

- **Only `static/` is public.** Templates and `theme.toml` are never served.
- **Images:** pictures from Media are sent as WebP to browsers that support
  it, at the same address. Give card images a `sizes` with their real width,
  so phones download the small copy.

## Shortcodes

Add `shortcodes/<name>.html` to make `[name ...]` work in posts. The template
gets `args` (values) and `params` (`key=value` pairs). Default's
`shortcodes/note.html` turns `[note "Text" kind="warning"]` into a box.

## Shared parts

A few parts are built into Bloogla, so every theme can use them without its
own copy: `templates/comments.html`, `templates/pagination.html`,
`templates/colors.html`, `templates/scheme_button.html`, and
`static/js/comments.js`, `scheme.js`, `code.js` and `prism.js` (code
highlighting). Include or link them as if they were in your theme:
`{% include "templates/comments.html" %}`,
`{{ theme_url }}/static/js/prism.js`.

To change one, put your own file at the same place in your theme: yours is
used instead. The originals are in `themes/_shared/` in Bloogla's source.

## Updating a bundled theme

Raise `version` in its `theme.toml`. On start, Bloogla replaces the older
copy and keeps it in `data/theme-backups/`. After changing a file in
`themes/_shared/`, raise the version of every bundled theme, so sites
reload them.
