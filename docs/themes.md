# Writing themes

Bloogla comes with seven themes, each a good starting point for your own:

| Theme      | Folder            | For                                                            |
| ---------- | ----------------- | -------------------------------------------------------------- |
| Default    | `themes/default`  | A clean blog: the newest post large, then a grid of cards      |
| Story      | `themes/story`    | Calm reading, in the style of Medium                           |
| Gazette    | `themes/gazette`  | A newspaper front page                                         |
| Folio      | `themes/folio`    | Photographs first: a large lead picture and a picture grid     |
| Notebook   | `themes/notebook` | Writing: one quiet column, titles listed by year               |
| Atelier    | `themes/atelier`  | Warm and friendly: rounded cards and a welcome card            |
| Brief      | `themes/brief`    | A business or project's news, with a strong logo header        |

A theme is a folder in `themes/` with a `theme.toml` and Tera templates in `templates/`. The easiest start is a copy of `themes/default` with a new folder name. Install a theme by copying its folder into `themes/` and restarting Bloogla; it then shows up in **Admin → Appearance**.

```toml
name = "My Theme"
version = "1.0.0"
author = "Your Name"
description = "One sentence about it."
preview_image = "static/preview.png"   # optional, shown in Admin → Appearance
```


| Template       | Used for                                           | Required |
| -------------- | -------------------------------------------------- | -------- |
| `index.html`   | Home page and search results                       | Yes      |
| `post.html`    | Single post                                        | Yes      |
| `tag.html`     | Posts with a tag                                   | Yes      |
| `page.html`    | Standalone pages (falls back to `post.html`)       | No       |
| `404.html`     | Not found page                                     | No       |

Templates refer to each other by their path inside the theme, so a copied theme works under any name: `{% extends "templates/layout.html" %}`, `{% include "templates/post_card.html" %}`. For the theme's own files use `theme_url`: `<link rel="stylesheet" href="{{ theme_url }}/static/css/style.css?v={{ asset_version }}">`.

A theme with a mistake (a template that doesn't parse, a missing required template, a broken `theme.toml`) is never used: Admin → Appearance shows what's wrong, and the site keeps using the default theme.

Lists of posts give each one a `post.url` (search results include pages too).

Every template gets `blog_name`, `blog_description`, `base_url`, `lang` (e.g. `tr`), `t` (translated interface text, e.g. `{{ t.back_to_all_posts }}`), `menu` (list of `label`/`url`), `blog_url`, `tags`, `search_query`, `show_views`, `seo_head`, `scheme_script`, `asset_version`, `theme_url` and `theme_options`. Put `{{ seo_head | safe }}` inside `<head>`: besides the SEO tags, it carries the verification tags the owner adds in **Settings → Site**, and on posts and pages with formulas, the KaTeX files that draw them (formulas are `<span data-math-style="inline">` or `"display"` in `content_html`).

Files in the theme's `static/` folder are served at `/theme-assets/<theme>/static/...`. Add `?v={{ asset_version }}` (the theme's version) to those URLs: browsers then keep them for a year, and a new theme version is picked up at once.

- `blog_url` is where the list of posts is: `/`, or `/blog` when the owner picks a page as the home page (**Settings → Reading**). Link "back to all posts" to it rather than to `/`. The home page itself is drawn with `page.html`, like any page.
- Listings also get `posts` and `pagination` (`current`, `total_pages`, `prev_url`, `next_url`).
- Single pages get `post` (with `post.author_name`, and `post.cover_width`/`post.cover_height` for images from the media library, and `post.cover_small` and `post.cover_thumb`, 800px- and 400px-wide copies for `srcset`; give cards a `sizes` with their real width so phones download the small one) and `content_html`. Pictures from the media library are sent as WebP to browsers that support it, under the same `.jpg`/`.png` address, so themes don't need to do anything for it.
- Posts also get `previous_post` and `next_post` (each with `title` and `url`, or empty for the oldest and newest post, and on pages): the older and newer post, for links at the end. `{% if previous_post %}` works everywhere.
- Posts with comments on also get `comments_enabled`, `comments`, `comment_action` and `comment_notice`; see the default theme's `comments.html`.
- A contact form (the writer's `[contact-form]`) arrives inside `content_html`, built by Bloogla with the comment form's classes (`comment-form`, `comment-fields`, `comment-trap`, `comment-notice`), so a theme that styles its comment form styles it too. The form also has the class `contact-form`, for anything a theme wants to change.

## Theme options

A theme can offer settings that admins change in **Admin → Appearance**, without touching code. List them in `theme.toml`; templates read them as `theme_options.<name>`:

```toml
[[options]]
name = "accent_color"
label = "Accent color"
type = "color"            # text, textarea, color, checkbox, select (with choices = [...]) or image
default = "#2563eb"
hint = "Used for links, buttons and highlights."
```

```html
<style>:root { --accent: {{ theme_options.accent_color }}; }</style>
{% if theme_options.show_powered_by %}Powered by Bloogla{% endif %}
```

Values are checked before they're saved (colors must look like `#2563eb`, images must be from the media library or `https://`, a select only accepts its choices), so they're safe to use in styles.

Every bundled theme offers the same six, so changing theme keeps the same kind of choices: **Logo** (`logo`, an image shown instead of the site title), **Light or dark** (`color_scheme`, a select: "Follow the visitor's device", "Always light" or "Always dark"), **Let visitors switch between light and dark** (`scheme_button`, a checkbox, on at first), **Accent color**, **Footer text** and **Show "Powered by Bloogla"**. Atelier adds a welcome card and Brief a heading and a menu button. Labels, hints and choices are English and are translated in the admin like any other text, so a bundled theme's new option needs a line in each language file (`cargo test` lists missing ones).

### Light and dark

The bundled themes put `data-scheme="light"` or `data-scheme="dark"` on `<html>` when "Always light" or "Always dark" is chosen, and write their dark colors for both cases:

```css
@media (prefers-color-scheme: dark) {
    :root:not([data-scheme="light"]) { --bg: #121214; /* ... */ }
}
:root[data-scheme="dark"] { --bg: #121214; /* ... */ }
```

Their `templates/colors.html` sets the accent color, and a lighter shade of it for dark mode (`color-mix(in srgb, <accent> 55%, white)`), so links stay readable on a dark background whatever color an admin picks. Copy it into your theme and include it in `<head>`.

Visitors can also choose for themselves with a button that goes from "same as the device" to "always light" to "always dark" and back: include `templates/scheme_button.html` in your header and copy `static/js/scheme.js` from a bundled theme (`colors.html` loads it, with `defer`). The button shows only when the site follows the visitor's device and `scheme_button` is on. Put `{{ scheme_script | safe }}` in `<head>`: these few lines put the visitor's saved choice on `<html>` as `data-scheme` before the page is drawn, so your dark colors written as above just work. They're in the page itself, so phones don't wait for one more file, and the page's security policy allows exactly that script (by its hash) and no other inline script. The choice stays in the visitor's browser (`localStorage`), never in a cookie. The button's labels are `t.scheme_device`, `t.scheme_light` and `t.scheme_dark`.

### Moving between pages

The bundled themes don't animate page changes: speed comes first. Browsers keep showing the old page until the new one is ready and then show it whole, so a click never waits on motion. If your theme adds a transition, keep it short and never hide or dim the new content while it plays.

The first big picture on a page (a post's cover, the newest post on the home page) uses `fetchpriority="high"` and `decoding="sync"`, so it's drawn together with the page instead of leaving an empty space for a moment. Pictures further down use `loading="lazy"` and `decoding="async"`.

## Scripts and the security policy

Theme pages are sent with a Content Security Policy: scripts, styles and fonts load from the site itself, images and media from any HTTPS address, and embeds from YouTube and Vimeo. Inline `<script>` blocks and `onclick=` attributes don't run, so put JavaScript in files under `static/js/`. If a theme needs more (web fonts, for example), list the extra sources in `theme.toml`:

```toml
[csp]
style-src = ["https://fonts.googleapis.com"]
font-src = ["https://fonts.gstatic.com"]
```

The default theme loads its syntax highlighter (`static/js/prism.js`, with about 20 common languages) only on pages that contain code.

## Theme shortcodes

Add `shortcodes/<name>.html` to a theme to make `[name ...]` available in posts. The template gets `args` (positional values) and `params` (`key=value` pairs). The default theme's `shortcodes/note.html` turns `[note "Text" kind="warning"]` into a highlighted box.

## Updating the bundled theme

Bump `version` in `theme.toml`. On start, Bloogla replaces an older installed copy and keeps the previous one in `data/theme-backups/`.
