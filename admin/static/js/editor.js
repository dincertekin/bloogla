// The post and page editor (admin/templates/post_editor.html): unsaved-text
// backup, preview, custom fields, earlier versions, images and keyboard saving.
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
        content.value = stored.content;
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
            content.value = revision.content;
            dirty = true;
            document.querySelector("[data-mode=write]").click();
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

    // Write / Preview
    document.querySelectorAll(".segmented [data-mode]").forEach((tab) => {
        tab.addEventListener("click", async () => {
            const showPreview = tab.dataset.mode === "preview";
            document.querySelectorAll(".segmented [data-mode]").forEach((t) =>
                t.setAttribute("aria-selected", String(t === tab))
            );
            content.hidden = showPreview;
            preview.hidden = !showPreview;
            if (!showPreview) return content.focus();

            preview.innerHTML = `<p class="text-muted">${text.rendering}</p>`;
            const response = await fetch("/admin/preview", {
                method: "POST",
                body: new URLSearchParams({ content: content.value }),
            });
            preview.innerHTML = response.ok
                ? await response.text()
                : `<p class="error-message">${text.previewFailed}</p>`;
        });
    });

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

    // "Choose" sets the cover image; "Insert image" adds Markdown at the cursor
    document.querySelector("[data-picker=cover]").addEventListener("click", () =>
        openMediaPicker((url) => setCover(url))
    );
    document.querySelector("[data-picker=insert]").addEventListener("click", () =>
        openMediaPicker((url, alt) => {
            const start = content.selectionStart;
            const before = content.value.slice(0, start);
            const spacer = before && !before.endsWith("\n") ? "\n\n" : "";
            content.setRangeText(`${spacer}![${alt}](${url})\n`, start, content.selectionEnd, "end");
            content.focus();
            dirty = true;
        })
    );
})();
