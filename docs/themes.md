# Writing themes

Bloogla comes with three themes: **Default** (`themes/default`), **Story**
(`themes/story`, a calm reading layout) and **Gazette** (`themes/gazette`, a
newspaper front page). Each is a good starting point for your own.

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

Every template gets `blog_name`, `blog_description`, `base_url`, `lang` (e.g. `tr`), `t` (translated interface text, e.g. `{{ t.back_to_all_posts }}`), `menu` (list of `label`/`url`), `tags`, `search_query`, `show_views`, `seo_head`, `asset_version`, `theme_url` and `theme_options`. Put `{{ seo_head | safe }}` inside `<head>`.

Files in the theme's `static/` folder are served at `/theme-assets/<theme>/static/...`. Add `?v={{ asset_version }}` (the theme's version) to those URLs: browsers then keep them for a year, and a new theme version is picked up at once.

- Listings also get `posts` and `pagination` (`current`, `total_pages`, `prev_url`, `next_url`).
- Single pages get `post` (with `post.author_name`, and `post.cover_width`/`post.cover_height` for images from the media library, and `post.cover_small`, an 800px-wide copy for `srcset`) and `content_html`.
- Posts with comments on also get `comments_enabled`, `comments`, `comment_action` and `comment_notice`; see the default theme's `comments.html`.

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

Values are checked before they're saved (colors must look like `#2563eb`, images must be from the media library or `https://`, a select only accepts its choices), so they're safe to use in styles. The default theme has three: accent color, footer text and "Powered by Bloogla".

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
