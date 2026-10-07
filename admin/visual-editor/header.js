// Adds the license header to the built bundle (run by `npm run build`).
const fs = require("fs");
const version = require("./node_modules/@tiptap/core/package.json").version;
const header = `/*! Bloogla visual editor: TipTap ${version} (ProseMirror) with Markdown. MIT licensed:
 * https://github.com/ueberdosis/tiptap and https://github.com/ProseMirror/prosemirror
 * Built from admin/visual-editor/entry.js; rebuild with: cd admin/visual-editor && npm install && npm run build
 */
`;
const body = "../static/js/visual-editor.body.js";
fs.writeFileSync("../static/js/visual-editor.js", header + fs.readFileSync(body, "utf8"));
fs.unlinkSync(body);
