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

// HTML the editor can't show as formatted text (video embeds, boxes, forms...),
// usually from WordPress imports. It runs from its first line to the next
// blank line, like an HTML block in Markdown.
const KEPT_HTML = /^<(iframe|video|audio|embed|object|script|style|svg|form|details|div|section|aside)[\s>/][\s\S]*?(?:\n[ \t]*\n|$)/i;

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
            const match = KEPT_HTML.exec(src);
            if (match) return { type: "htmlBlock", raw: match[0], text: match[0].trim() };
        },
    },
    parseMarkdown(token) {
        return { type: "htmlBlock", attrs: { html: token.text } };
    },
    renderMarkdown(node) {
        return node.attrs.html;
    },
});

window.BlooglaVisualEditor = {
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
