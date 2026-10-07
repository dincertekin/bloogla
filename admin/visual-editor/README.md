# Visual editor source

`admin/static/js/visual-editor.js` is built from `entry.js` here: the
[TipTap](https://tiptap.dev) editor (built on ProseMirror) with its Markdown
extension, plus a small "shortcode" block so `[youtube URL]`-style shortcodes
stay exactly as written.

The built file is committed and embedded in the Bloogla binary, so you only
need Node.js to change or update the editor:

```bash
cd admin/visual-editor
npm install
npm run build
```

Posts are always stored as Markdown; this editor only changes how they're
written. `admin/static/js/editor.js` connects it to the editor page.
