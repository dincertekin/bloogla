// The post and page editor (admin/templates/post_editor.html): the visual
// editor and preview, formatting buttons, unsaved-text backup, custom fields,
// earlier versions, images and keyboard saving.
//
// Posts are saved as Markdown from a hidden text area (#content). The visual
// editor (admin/static/js/visual-editor.js) writes into it as you type.
(() => {
    const form = document.getElementById("editor-form");
    if (!form) return;
    // Translated messages, from data-text-* attributes on the form.
    const text = {
        unsavedFrom: form.dataset.textUnsavedFrom,
        loadedVersion: form.dataset.textLoadedVersion,
        dismiss: form.dataset.textDismiss,
        rendering: form.dataset.textRendering,
        previewFailed: form.dataset.textPreviewFailed,
        linkPrompt: form.dataset.textLinkPrompt,
        videoPrompt: form.dataset.textVideoPrompt,
        visualPlaceholder: form.dataset.textVisualPlaceholder,
    };
    const content = document.getElementById("content");
    const preview = document.getElementById("preview");
    const status = document.getElementById("status");
    let dirty = false;

    const titleField = form.querySelector("[name=title]");

    // Keep a copy of unsaved text in this browser, in case the tab closes.
    const draftKey = "bloogla:draft:" + form.getAttribute("action");
    let draftTimer = null;
    const saveDraft = () => {
        clearTimeout(draftTimer);
        draftTimer = setTimeout(() => {
            try {
                localStorage.setItem(draftKey, JSON.stringify({
                    title: titleField.value, content: content.value, at: Date.now(),
                }));
            } catch { /* storage full or disabled */ }
        }, 800);
    };
    const clearDraft = () => {
        clearTimeout(draftTimer);
        try { localStorage.removeItem(draftKey); } catch { }
    };

    const banner = document.getElementById("draft-banner");
    let stored = null;
    try { stored = JSON.parse(localStorage.getItem(draftKey)); } catch { }
    if (new URLSearchParams(location.search).has("saved")) {
        clearDraft();
    } else if (stored && (stored.title !== titleField.value || stored.content !== content.value)) {
        const when = new Date(stored.at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
        document.getElementById("draft-text").textContent = text.unsavedFrom.replace("{when}", when);
        banner.hidden = false;
    }
    document.getElementById("draft-restore").addEventListener("click", () => {
        titleField.value = stored.title;
        setMarkdown(stored.content);
        banner.hidden = true;
        dirty = true;
    });
    document.getElementById("draft-discard").addEventListener("click", () => {
        clearDraft();
        banner.hidden = true;
    });

    // Unsaved changes warning
    form.addEventListener("input", () => {
        dirty = true;
        saveDraft();
    });
    form.addEventListener("submit", () => {
        dirty = false;
        clearDraft();
    });

    // Custom fields
    const fieldRows = document.getElementById("field-rows");
    document.getElementById("add-field").addEventListener("click", () => {
        const row = document.getElementById("field-row-template").content.cloneNode(true);
        fieldRows.appendChild(row);
        fieldRows.lastElementChild.querySelector("input").focus();
        dirty = true;
    });
    fieldRows.addEventListener("click", (event) => {
        const remove = event.target.closest("[data-remove-field]");
        if (!remove) return;
        remove.closest(".field-row").remove();
        dirty = true;
    });

    // Earlier versions: load into the editor; saving keeps it.
    document.querySelectorAll("[data-revision]").forEach((button) => {
        button.addEventListener("click", async () => {
            const response = await fetch(`/admin/posts/${form.dataset.postId}/revisions/${button.dataset.revision}`);
            if (!response.ok) return;
            const revision = await response.json();
            titleField.value = revision.title;
            setMarkdown(revision.content);
            dirty = true;
            showPreview(false);
            banner.hidden = false;
            document.getElementById("draft-text").textContent =
                text.loadedVersion.replace("{date}", button.dataset.revisionLabel);
            document.getElementById("draft-restore").hidden = true;
            document.getElementById("draft-discard").textContent = text.dismiss;
            window.scrollTo({ top: 0, behavior: "smooth" });
        });
    });
    window.addEventListener("beforeunload", (event) => {
        if (dirty && !window.skipUnloadWarning) event.preventDefault();
    });

    // Ctrl+S / Cmd+S saves
    document.addEventListener("keydown", (event) => {
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
            event.preventDefault();
            form.requestSubmit();
        }
    });

    // Show the address a new post will get while its title is typed
    const titleInput = form.querySelector("[name=title]");
    const slugInput = form.querySelector("[name=slug]");
    const toSlug = (text) =>
        text
            .toLocaleLowerCase("tr")
            .replace(/ı/g, "i")
            .replace(/ß/g, "ss")
            .normalize("NFD")
            .replace(/[\u0300-\u036f]/g, "")
            .replace(/[^\p{L}\p{N}]+/gu, "-")
            .replace(/^-+|-+$/g, "");
    slugInput.dataset.fallback = slugInput.placeholder;
    if (!slugInput.value) {
        titleInput.addEventListener("input", () => {
            slugInput.placeholder = toSlug(titleInput.value) || slugInput.dataset.fallback;
        });
    }

    // The publish date only matters when scheduling or backdating
    const dateGroup = document.getElementById("date-group");
    const syncDate = () => (dateGroup.hidden = status.value === "draft");
    status.addEventListener("change", syncDate);
    syncDate();

    const coverInput = document.getElementById("cover_image");
    const coverPreview = document.getElementById("cover-preview");
    const coverEmpty = document.getElementById("cover-empty");
    const coverRemove = document.getElementById("cover-remove");

    function setCover(url) {
        coverInput.value = url;
        coverPreview.src = url;
        coverPreview.hidden = !url;
        coverEmpty.hidden = !!url;
        coverRemove.hidden = !url;
        dirty = true;
    }
    coverRemove.addEventListener("click", () => setCover(""));

    // ---- The visual editor and the preview ----

    const formatBar = document.querySelector(".format-bar");
    const visualBox = document.getElementById("visual-editor");
    const tabs = document.querySelectorAll(".segmented [data-mode]");

    const editor = window.BlooglaVisualEditor.create({
        element: visualBox,
        markdown: content.value,
        placeholder: text.visualPlaceholder,
        onChange: (markdown) => {
            content.value = markdown;
            // Marks the post as changed and backs it up, like typing does.
            content.dispatchEvent(new Event("input", { bubbles: true }));
        },
    });

    // Replace the whole text (restoring a draft or an earlier version).
    function setMarkdown(markdown) {
        content.value = markdown;
        editor.commands.setContent(markdown, { contentType: "markdown", emitUpdate: false });
    }

    async function showPreview(on) {
        tabs.forEach((tab) => tab.setAttribute("aria-selected", String((tab.dataset.mode === "preview") === on)));
        visualBox.hidden = on;
        formatBar.hidden = on;
        preview.hidden = !on;
        if (!on) return;

        preview.innerHTML = `<p class="text-muted">${text.rendering}</p>`;
        const response = await fetch("/admin/preview", {
            method: "POST",
            body: new URLSearchParams({ content: content.value }),
        });
        preview.innerHTML = response.ok
            ? await response.text()
            : `<p class="error-message">${text.previewFailed}</p>`;
    }

    tabs.forEach((tab) =>
        tab.addEventListener("click", () => {
            showPreview(tab.dataset.mode === "preview");
            if (tab.dataset.mode !== "preview") editor.commands.focus();
        })
    );

    // ---- Formatting buttons ----

    const run = () => editor.chain().focus();
    const formats = {
        // Paragraph → big heading → smaller heading → paragraph
        heading: () => {
            if (editor.isActive("heading", { level: 2 })) run().setHeading({ level: 3 }).run();
            else if (editor.isActive("heading", { level: 3 })) run().setParagraph().run();
            else run().setHeading({ level: 2 }).run();
        },
        bold: () => run().toggleBold().run(),
        italic: () => run().toggleItalic().run(),
        // Code inside a sentence, or a code block for several lines.
        code: () => {
            const { $from, $to, empty } = editor.state.selection;
            const block = $from.parent !== $to.parent || (empty && $from.parent.textContent === "");
            if (block || editor.isActive("codeBlock")) run().toggleCodeBlock().run();
            else run().toggleCode().run();
        },
        link: () => {
            const answer = window.prompt(text.linkPrompt, editor.getAttributes("link").href || "https://");
            if (answer === null) return editor.commands.focus();
            const url = answer.trim();
            if (!url || url === "https://") return run().extendMarkRange("link").unsetLink().run();
            if (editor.state.selection.empty && !editor.isActive("link")) {
                return run().insertContent({ type: "text", text: url, marks: [{ type: "link", attrs: { href: url } }] }).run();
            }
            run().extendMarkRange("link").setLink({ href: url }).run();
        },
        // Videos are shortcodes: [youtube URL] or [vimeo URL].
        video: () => {
            const url = (window.prompt(text.videoPrompt, "") || "").trim();
            if (!url) return editor.commands.focus();
            const name = /vimeo\.com/.test(url) ? "vimeo" : "youtube";
            run().insertContent({ type: "shortcode", attrs: { text: `[${name} ${url}]` } }).run();
            // No cover yet: use the YouTube video's thumbnail.
            const youtube = url.match(/(?:youtu\.be\/|[?&]v=|\/embed\/|\/shorts\/)([\w-]{11})/);
            if (youtube && !coverInput.value) setCover(`https://i.ytimg.com/vi/${youtube[1]}/hqdefault.jpg`);
        },
        bullets: () => run().toggleBulletList().run(),
        numbers: () => run().toggleOrderedList().run(),
        quote: () => run().toggleBlockquote().run(),
        divider: () => run().setHorizontalRule().run(),
    };

    // Buttons show what's on at the cursor (bold, a list...).
    const pressedWhen = {
        heading: () => editor.isActive("heading"),
        bold: () => editor.isActive("bold"),
        italic: () => editor.isActive("italic"),
        code: () => editor.isActive("code") || editor.isActive("codeBlock"),
        link: () => editor.isActive("link"),
        bullets: () => editor.isActive("bulletList"),
        numbers: () => editor.isActive("orderedList"),
        quote: () => editor.isActive("blockquote"),
    };
    editor.on("transaction", () => {
        formatBar.querySelectorAll("[data-format]").forEach((button) => {
            const check = pressedWhen[button.dataset.format];
            button.setAttribute("aria-pressed", String(check !== undefined && check()));
        });
    });

    // Clicking a button keeps the cursor in the writing area, so typing goes on.
    formatBar.addEventListener("mousedown", (event) => {
        if (event.target.closest("button")) event.preventDefault();
    });

    formatBar.addEventListener("click", (event) => {
        const button = event.target.closest("[data-format]");
        if (button) formats[button.dataset.format]();
    });

    // Cmd/Ctrl+K adds a link (bold and italic are built into the editor).
    visualBox.addEventListener("keydown", (event) => {
        if ((event.ctrlKey || event.metaKey) && !event.shiftKey && event.key.toLowerCase() === "k") {
            event.preventDefault();
            formats.link();
        }
    });

    // Tooltips show ⌘ on Macs and Ctrl+ elsewhere.
    if (!/Mac|iPhone|iPad/.test(navigator.platform)) {
        formatBar.querySelectorAll("[title*='⌘']").forEach((b) => (b.title = b.title.replace("⌘", "Ctrl+")));
    }

    // "Choose" sets the cover image; "Insert image" adds one at the cursor.
    document.querySelector("[data-picker=cover]").addEventListener("click", () =>
        openMediaPicker((url) => setCover(url))
    );
    document.querySelector("[data-picker=insert]").addEventListener("click", () =>
        openMediaPicker((url, alt) => run().setImage({ src: url, alt }).run())
    );
})();
