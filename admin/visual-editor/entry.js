// Bloogla's visual editor: TipTap (ProseMirror) reading and writing Markdown.
// Built from this file into admin/static/js/visual-editor.js (see the header
// of that file); used by admin/static/js/editor.js.
import { Editor, Node, mergeAttributes } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import Placeholder from "@tiptap/extension-placeholder";
import { TableKit } from "@tiptap/extension-table";
import { TaskList, TaskItem } from "@tiptap/extension-list";
import { Markdown } from "@tiptap/markdown";
import { Selection } from "@tiptap/pm/state";
import { Marked } from "marked";

// A shortcode on its own line, like [youtube URL], [toc] or [note "Text"].
const SHORTCODE_LINE = /^\[([a-z][\w-]*)(?:[ \t][^\n\]]*)?\][ \t]*(?:\n|$)/i;

// Shortcodes are kept exactly as written: shown as a labelled block in the
// editor and written back to Markdown unchanged.
const Shortcode = Node.create({
    name: "shortcode",
    group: "block",
    atom: true,
    selectable: true,
    draggable: true,
    addAttributes() {
        return { text: { default: "" } };
    },
    parseHTML() {
        return [{ tag: "div[data-shortcode]", getAttrs: (el) => ({ text: el.getAttribute("data-shortcode") }) }];
    },
    renderHTML({ node, HTMLAttributes }) {
        return ["div", mergeAttributes(HTMLAttributes, { "data-shortcode": node.attrs.text, class: "shortcode-block" }), node.attrs.text];
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

// The editor writes the whole post back as Markdown in its own style, so
// `*` lists become `-` and so on, and what it can't show (like <kbd> or
// <sup>) is lost. To tell a harmless change of style from a loss, both texts
// are turned into HTML (with spacing evened out) and compared.
const reference = new Marked({ gfm: true });
const asHtml = (markdown) =>
    reference
        .parse(markdown, { async: false })
        .replace(/>\s+</g, "><")
        .replace(/\s+/g, " ")
        .trim();

window.BlooglaVisualEditor = {
    // Put `markdown` in the editor. Returns false when the editor would
    // change what the post says or how it looks, so it should be edited as
    // Markdown instead.
    load(editor, markdown) {
        editor.commands.setContent(markdown, { contentType: "markdown", emitUpdate: false });
        // Start at the beginning, like a newly created editor. (Left at the
        // end, a [youtube …] block there would be selected, and typing
        // would replace it.)
        editor.commands.command(({ tr }) => {
            tr.setSelection(Selection.atStart(tr.doc));
            return true;
        });
        return asHtml(editor.getMarkdown()) === asHtml(markdown);
    },

    create({ element, markdown, placeholder, onChange }) {
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
                Shortcode,
                HtmlBlock,
                Markdown,
            ],
            content: markdown,
            contentType: "markdown",
            onUpdate: ({ editor }) => onChange(editor.getMarkdown()),
        });
    },
};
