// Bloogla's visual editor: TipTap (ProseMirror) reading and writing Markdown.
// Built from this file into admin/static/js/visual-editor.js (see the header
// of that file); used by admin/static/js/editor.js.
import { Editor, Node, InputRule, mergeAttributes } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import Placeholder from "@tiptap/extension-placeholder";
import { TableKit } from "@tiptap/extension-table";
import { TaskList, TaskItem } from "@tiptap/extension-list";
import { Markdown } from "@tiptap/markdown";
import Subscript from "@tiptap/extension-subscript";
import Superscript from "@tiptap/extension-superscript";

// A shortcode on its own line, like [youtube URL], [toc] or [note "Text"].
const SHORTCODE_LINE = /^\[([a-z][\w-]*)(?:[ \t][^\n\]]*)?\][ \t]*(?:\n|$)/i;

// Shortcodes are kept exactly as written: shown as a block in the editor
// and written back to Markdown unchanged. Those with a label (translated,
// by name, e.g. "contact-form") show it, and their title="…", instead of
// their text.
const Shortcode = Node.create({
    name: "shortcode",
    group: "block",
    atom: true,
    selectable: true,
    draggable: true,
    addOptions() {
        return { labels: {} };
    },
    addAttributes() {
        return { text: { default: "" } };
    },
    parseHTML() {
        return [{ tag: "div[data-shortcode]", getAttrs: (el) => ({ text: el.getAttribute("data-shortcode") }) }];
    },
    renderHTML({ node, HTMLAttributes }) {
        const name = node.attrs.text.match(/^\[([\w-]+)/)?.[1];
        const label = this.options.labels[name];
        const title = node.attrs.text.match(/title="([^"]*)"/)?.[1];
        const shown = label ? (title ? `${label}: ${title}` : label) : node.attrs.text;
        const attributes = { "data-shortcode": node.attrs.text, class: label ? "shortcode-block is-labelled" : "shortcode-block" };
        return ["div", mergeAttributes(HTMLAttributes, attributes), shown];
    },
    markdownTokenName: "shortcode",
    markdownTokenizer: {
        name: "shortcode",
        level: "block",
        start: (src) => {
            const match = /(^|\n)\[[a-z]/i.exec(src);
            return match ? match.index + match[1].length : -1;
        },
        tokenize(src) {
            const match = SHORTCODE_LINE.exec(src);
            if (match) return { type: "shortcode", raw: match[0], text: match[0].trim() };
        },
    },
    parseMarkdown(token) {
        return { type: "shortcode", attrs: { text: token.text } };
    },
    renderMarkdown(node) {
        return node.attrs.text;
    },
});

// HTML the editor can't show as formatted text (video embeds, boxes, forms,
// centered images...), usually from WordPress imports.
const KEPT_HTML_START = /^<(iframe|video|audio|embed|object|script|style|svg|form|details|div|section|aside|figure|picture|table|center|p|img)[\s>/]/i;

// The HTML block at the start of `src`, or null. It runs to the line with its
// closing tag, so a box like <details> with blank lines inside stays whole
// (ending it at the first blank line lost the closing tag, and the published
// post then hid everything after it). Without a closing tag (an <img>, or
// unfinished HTML) it runs to the next blank line.
function keptHtml(src) {
    const start = KEPT_HTML_START.exec(src);
    if (!start) return null;
    // Count tags with the same name, so a <div> inside a <div> doesn't end it.
    const sameTag = new RegExp(`<(/?)${start[1]}(?=[\\s>/])`, "gi");
    let depth = 0;
    for (let tag; (tag = sameTag.exec(src)); ) {
        depth += tag[1] ? -1 : 1;
        if (depth === 0) {
            const lineEnd = src.indexOf("\n", tag.index);
            return lineEnd === -1 ? src : src.slice(0, lineEnd + 1);
        }
    }
    const blankLine = /\n[ \t]*\n/.exec(src);
    return blankLine ? src.slice(0, blankLine.index + 1) : src;
}

// Kept exactly as written, shown as a block of code (like shortcodes).
const HtmlBlock = Node.create({
    name: "htmlBlock",
    group: "block",
    atom: true,
    selectable: true,
    draggable: true,
    addAttributes() {
        return { html: { default: "" } };
    },
    parseHTML() {
        return [{ tag: "div[data-html-block]", getAttrs: (el) => ({ html: el.textContent }) }];
    },
    renderHTML({ node }) {
        return ["div", { "data-html-block": "", class: "shortcode-block html-block" }, node.attrs.html];
    },
    markdownTokenName: "htmlBlock",
    markdownTokenizer: {
        name: "htmlBlock",
        level: "block",
        start: (src) => {
            const match = /(^|\n)<[a-z]/i.exec(src);
            return match ? match.index + match[1].length : -1;
        },
        tokenize(src) {
            const html = keptHtml(src);
            if (html) return { type: "htmlBlock", raw: html, text: html.trim() };
        },
    },
    parseMarkdown(token) {
        return { type: "htmlBlock", attrs: { html: token.text } };
    },
    renderMarkdown(node) {
        return node.attrs.html;
    },
});

// Subscript and superscript (H₂O, mc²). Markdown has no way to write them,
// so they're saved as <sub>/<sup>, which the site shows as they are.
const SubscriptMark = Subscript.extend({
    renderMarkdown: (node, h) => `<sub>${h.renderChildren(node)}</sub>`,
});
const SuperscriptMark = Superscript.extend({
    renderMarkdown: (node, h) => `<sup>${h.renderChildren(node)}</sup>`,
});

// Formulas in LaTeX are kept exactly as written: Markdown's escaping would
// otherwise double backslashes and lose `\,` or `_`. A block between $$
// lines is edited like code; $…$ inside a sentence is kept as one piece
// (delete it and type it again to change it).

// A $$ … $$ block at the start of `src`, or null.
const MATH_BLOCK = /^\$\$[ \t]*\n([\s\S]*?)\n\$\$[ \t]*(?:\n|$)/;
// $…$ in a sentence: not "$ " at the start or " $" at the end, and not
// followed by a digit, so prices like "$5 and $10" stay text.
const MATH_INLINE = /^\$(?![\s$])((?:\\.|[^$\\\n])+?)(?<!\s)\$(?!\d)/;

const MathBlock = Node.create({
    name: "mathBlock",
    group: "block",
    content: "text*",
    marks: "",
    code: true,
    defining: true,
    parseHTML() {
        return [{ tag: "pre[data-math-block]", preserveWhitespace: "full" }];
    },
    renderHTML() {
        return ["pre", { "data-math-block": "", class: "math-block" }, ["code", 0]];
    },
    markdownTokenName: "mathBlock",
    markdownTokenizer: {
        name: "mathBlock",
        level: "block",
        start: (src) => {
            const match = /(^|\n)\$\$/.exec(src);
            return match ? match.index + match[1].length : -1;
        },
        tokenize(src) {
            const match = MATH_BLOCK.exec(src);
            if (match) return { type: "mathBlock", raw: match[0], text: match[1] };
        },
    },
    parseMarkdown(token) {
        return { type: "mathBlock", content: token.text ? [{ type: "text", text: token.text }] : [] };
    },
    // The text as typed: the pieces joined, never escaped.
    renderMarkdown(node) {
        const latex = (node.content || []).map((part) => part.text || "").join("");
        return `$$\n${latex}\n$$`;
    },
    // Typing $$ and a space on an empty line starts a formula block.
    addInputRules() {
        return [
            new InputRule({
                find: /^\$\$\s$/,
                handler: ({ range, chain }) => {
                    chain().deleteRange(range).setNode("mathBlock").run();
                },
            }),
        ];
    },
});

const MathInline = Node.create({
    name: "mathInline",
    group: "inline",
    inline: true,
    atom: true,
    selectable: true,
    addAttributes() {
        return { latex: { default: "" } };
    },
    parseHTML() {
        return [{ tag: "span[data-math-inline]", getAttrs: (el) => ({ latex: el.getAttribute("data-math-inline") }) }];
    },
    renderHTML({ node }) {
        return ["span", { "data-math-inline": node.attrs.latex, class: "math-inline" }, `$${node.attrs.latex}$`];
    },
    markdownTokenName: "mathInline",
    markdownTokenizer: {
        name: "mathInline",
        level: "inline",
        start: (src) => src.indexOf("$"),
        tokenize(src) {
            const match = MATH_INLINE.exec(src);
            if (match) return { type: "mathInline", raw: match[0], text: match[1] };
        },
    },
    parseMarkdown(token) {
        return { type: "mathInline", attrs: { latex: token.text } };
    },
    renderMarkdown(node) {
        return `$${node.attrs.latex}$`;
    },
    // Typing $…$ in a sentence keeps it as a formula.
    addInputRules() {
        return [
            new InputRule({
                find: /(?:^|\s)\$([^$\s](?:[^$]*[^$\s])?)\$$/,
                handler: ({ state, range, match }) => {
                    const start = range.from + match[0].indexOf("$");
                    state.tr.replaceWith(start, range.to, this.type.create({ latex: match[1] }));
                },
            }),
        ];
    },
});

window.BlooglaVisualEditor = {
    // `shortcodeLabels`: what some shortcodes show in the editor, by name,
    // e.g. { "contact-form": "Contact form" }.
    create({ element, markdown, placeholder, shortcodeLabels, onChange }) {
        return new Editor({
            element,
            extensions: [
                StarterKit.configure({
                    // Markdown has no underline.
                    underline: false,
                    link: { openOnClick: false, autolink: true },
                }),
                Image,
                Placeholder.configure({ placeholder }),
                TableKit,
                TaskList,
                TaskItem.configure({ nested: true }),
                Shortcode.configure({ labels: shortcodeLabels || {} }),
                HtmlBlock,
                SubscriptMark,
                SuperscriptMark,
                MathBlock,
                MathInline,
                Markdown,
            ],
            content: markdown,
            contentType: "markdown",
            onUpdate: ({ editor }) => onChange(editor.getMarkdown()),
        });
    },
};
