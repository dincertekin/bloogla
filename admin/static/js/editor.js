// The post and page editor (admin/templates/post_editor.html): the visual
// editor, the "Plain text" tab and the preview, formatting buttons,
// unsaved-text backup, earlier versions, images and keyboard saving.
//
// Posts are saved as Markdown from a text area (#content). The visual editor
// (admin/static/js/visual-editor.js) writes into it as you type; the
// "Plain text" tab shows the text area itself.
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
        saved: form.dataset.textSaved,
        saveFailed: form.dataset.textSaveFailed,
    };
    const content = document.getElementById("content");
    const preview = document.getElementById("preview");
    const status = document.getElementById("status");
    let dirty = false;

    const titleField = form.querySelector("[name=title]");

    // Keep a copy of unsaved text in this browser, in case the tab closes.
    // A new post gets its own address once saved, so the key is read each time.
    const draftKey = () => "bloogla:draft:" + form.getAttribute("action");
    let draftTimer = null;
    const saveDraft = () => {
        clearTimeout(draftTimer);
        draftTimer = setTimeout(() => {
            try {
                localStorage.setItem(draftKey(), JSON.stringify({
                    title: titleField.value, content: content.value, at: Date.now(),
                }));
            } catch { /* storage full or disabled */ }
        }, 800);
    };
    const clearDraft = () => {
        clearTimeout(draftTimer);
        try { localStorage.removeItem(draftKey()); } catch { }
    };

    const banner = document.getElementById("draft-banner");
    let stored = null;
    try { stored = JSON.parse(localStorage.getItem(draftKey())); } catch { }
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

    // Earlier versions: load into the editor; saving keeps it. (One listener
    // for all buttons, because saving replaces the list.)
    document.addEventListener("click", async (event) => {
        const button = event.target.closest("[data-revision]");
        if (!button) return;
        const response = await fetch(`/admin/posts/${form.dataset.postId}/revisions/${button.dataset.revision}`);
        if (!response.ok) return;
        const revision = await response.json();
        titleField.value = revision.title;
        setMarkdown(revision.content);
        dirty = true;
        banner.hidden = false;
        document.getElementById("draft-text").textContent =
            text.loadedVersion.replace("{date}", button.dataset.revisionLabel);
        document.getElementById("draft-restore").hidden = true;
        document.getElementById("draft-discard").textContent = text.dismiss;
        window.scrollTo({ top: 0, behavior: "smooth" });
    });
    window.addEventListener("beforeunload", (event) => {
        if (dirty && !window.skipUnloadWarning) event.preventDefault();
    });

    // Saving happens in the background: the page stays where it is, and the
    // parts that changed (message, tags, earlier versions) are refreshed from
    // the editor page the server answers with. Without JavaScript the form
    // is sent the ordinary way.
    const saveButton = form.querySelector("[type=submit]");
    const saveNote = form.querySelector("[data-save-note]");
    const noteText = saveNote.textContent;
    let saving = false;
    const showNote = (message) => {
        saveNote.textContent = message;
        clearTimeout(showNote.timer);
        showNote.timer = setTimeout(() => (saveNote.textContent = noteText), 4000);
    };
    const showError = (html) => {
        document.getElementById("save-status").innerHTML = html;
        showNote(document.getElementById("save-status").textContent.trim() || text.saveFailed);
    };
    form.addEventListener("submit", async (event) => {
        if (event.defaultPrevented) return;
        event.preventDefault();
        if (saving) return;
        saving = true;
        saveButton.classList.add("is-busy");
        try {
            const response = await fetch(form.action, {
                method: "POST",
                body: new URLSearchParams(new FormData(form)),
            });
            const page = new DOMParser().parseFromString(await response.text(), "text/html");
            const address = new URL(response.url);
            if (!response.ok || !address.searchParams.has("saved")) {
                // The editor came back with a message, like "Add a title first."
                const message = page.getElementById("save-status");
                showError(message && message.textContent.trim()
                    ? message.innerHTML
                    : `<div class="alert alert-error" role="alert">${text.saveFailed}</div>`);
                return;
            }
            clearDraft();
            dirty = false;
            // A new post now has its own address; the server may also have
            // filled in or tidied the web address.
            const saved = page.getElementById("editor-form");
            form.setAttribute("action", saved.getAttribute("action"));
            form.dataset.postId = saved.dataset.postId;
            slugInput.value = saved.querySelector("[name=slug]").value;
            showAddress();
            address.searchParams.delete("saved");
            history.replaceState(null, "", address.pathname + address.search);
            replaceLiveParts(page);
            showNote(text.saved);
        } catch {
            showError(`<div class="alert alert-error" role="alert">${text.saveFailed}</div>`);
        } finally {
            saving = false;
            saveButton.classList.remove("is-busy");
        }
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
            showAddress();
        });
    }

    // The address is shown as text ("Address: example.com/post/my-title Edit");
    // Edit turns it into the field, Done (or Enter) turns it back.
    const addressView = form.querySelector("[data-address-view]");
    const addressEditor = form.querySelector("[data-address-editor]");
    const addressSlug = form.querySelector("[data-address-slug]");
    const doneButton = form.querySelector("[data-address-done]");
    function showAddress() {
        const slug = toSlug(slugInput.value) || slugInput.placeholder;
        addressSlug.textContent = slug;
    }
    function editAddress(editing) {
        addressView.hidden = editing;
        addressEditor.hidden = !editing;
        doneButton.hidden = !editing;
        if (editing) slugInput.focus();
        else showAddress();
    }
    form.querySelector("[data-address-edit]").addEventListener("click", () => editAddress(true));
    doneButton.addEventListener("click", () => editAddress(false));
    slugInput.addEventListener("keydown", (event) => {
        if (event.key === "Enter") {
            event.preventDefault();
            editAddress(false);
        }
    });
    editAddress(false);

    // Tags: typing a name and pressing Enter (or a comma) ticks the tag with
    // that name, or adds a new one that's created when the post is saved.
    // (The list is looked up each time, because saving replaces it.)
    const tagInput = form.querySelector("[data-new-tag]");
    if (tagInput) {
        const addTag = () => {
            const tagList = form.querySelector("[data-tag-list]");
            const name = tagInput.value.replace(/,/g, " ").replace(/\s+/g, " ").trim();
            tagInput.value = "";
            if (!name) return;
            const same = [...tagList.querySelectorAll(".tag-chip")].find(
                (chip) => chip.textContent.trim().toLocaleLowerCase() === name.toLocaleLowerCase()
            );
            if (same) {
                same.querySelector("input").checked = true;
            } else {
                const chip = document.createElement("label");
                chip.className = "tag-chip";
                const box = document.createElement("input");
                box.type = "checkbox";
                box.name = "new_tags";
                box.value = name;
                box.checked = true;
                chip.append(box, " ", name);
                tagList.appendChild(chip);
            }
            tagList.hidden = false;
            dirty = true;
            saveDraft();
        };
        tagInput.addEventListener("keydown", (event) => {
            if (event.key === "Enter" || event.key === ",") {
                event.preventDefault();
                addTag();
            }
        });
        // A name typed but not added yet still counts when saving.
        form.addEventListener("submit", addTag, true);
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

    // ---- The visual editor, the "Plain text" tab and the preview ----
    //
    // The visual editor (the Write tab) writes the whole post back in its own
    // Markdown, and drops what it can't show (some HTML, <kbd>, <sup>...).
    // So it only takes a post it can write back without changing how it
    // looks; any other post opens in the "Plain text" tab (the Markdown
    // itself), with a note saying why, and stays exactly as written unless
    // the person chooses Write anyway.

    const formatBar = document.querySelector(".format-bar");
    const visualBox = document.getElementById("visual-editor");
    const tabs = document.querySelectorAll(".segmented [data-mode]");
    const notice = document.getElementById("markdown-notice");
    let mode = "write";
    // Whether the visual editor holds the text in #content. Typing in the
    // "Plain text" tab makes it out of date until the visual editor loads it again.
    let visualIsCurrent = false;

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
    content.addEventListener("input", (event) => {
        if (event.isTrusted) visualIsCurrent = false;
    });

    // Show one tab: "write" (the visual editor), "markdown" (the "Plain
    // text" tab) or "preview".
    function setMode(next) {
        mode = next;
        tabs.forEach((tab) => tab.setAttribute("aria-selected", String(tab.dataset.mode === next)));
        visualBox.hidden = next !== "write";
        formatBar.hidden = next !== "write";
        content.hidden = next !== "markdown";
        preview.hidden = next !== "preview";
        notice.hidden = true;
        if (next === "preview") showPreview();
    }

    // Open the visual editor, unless it would change the post: then open
    // the "Plain text" tab with the note instead (`anyway` skips that check).
    // Returns whether the visual editor opened.
    function openVisual(anyway) {
        if (!visualIsCurrent) {
            const keeps = window.BlooglaVisualEditor.load(editor, content.value);
            if (!keeps && !anyway) {
                setMode("markdown");
                notice.hidden = false;
                return false;
            }
            visualIsCurrent = true;
        }
        setMode("write");
        return true;
    }

    // Replace the whole text (restoring a draft or an earlier version).
    function setMarkdown(markdown) {
        content.value = markdown;
        visualIsCurrent = false;
        if (mode === "markdown") return;
        openVisual(false);
    }

    async function showPreview() {
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
            if (tab.dataset.mode !== "write") {
                setMode(tab.dataset.mode);
                if (mode === "markdown") content.focus();
            } else if (openVisual(false)) {
                editor.commands.focus();
            }
        })
    );
    notice.querySelector("[data-visual-anyway]").addEventListener("click", () => {
        openVisual(true);
        editor.commands.focus();
    });
    openVisual(false);

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
