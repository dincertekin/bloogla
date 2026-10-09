// The settings page (admin/templates/settings.html): choosing the site icon,
// the menu editor, the email provider list and the time zone.
(() => {
    const input = document.getElementById("site_icon");
    if (!input) return;
    const preview = document.getElementById("icon-preview");
    const empty = document.getElementById("icon-empty");
    const remove = document.getElementById("icon-remove");
    const setIcon = (url) => {
        input.value = url;
        preview.src = url;
        preview.hidden = !url;
        empty.hidden = !!url;
        remove.hidden = !url;
    };
    document.getElementById("icon-choose").addEventListener("click", () => openMediaPicker(setIcon));
    remove.addEventListener("click", () => setIcon(""));
})();

// Menu editor: one row per link (name and address) instead of typing
// "Name | /address" lines. It writes those lines into the hidden #nav_menu
// text box, which is what the form sends.
(() => {
    const textarea = document.getElementById("nav_menu");
    const editor = document.getElementById("menu-editor");
    if (!textarea || !editor) return;
    const rows = document.getElementById("menu-rows");
    const empty = document.getElementById("menu-empty");
    const template = document.getElementById("menu-row-template");
    const suggestions = [...document.querySelectorAll("#menu-links option")];

    const save = () => {
        textarea.value = [...rows.children]
            .map((row) => {
                // "|" separates the name from the address, so it can't be in the name.
                const label = row.querySelector("[data-menu-label]").value.replace(/\|/g, "").trim();
                const url = row.querySelector("[data-menu-url]").value.trim();
                return label || url ? `${label} | ${url}` : "";
            })
            .filter(Boolean)
            .join("\n");
        empty.hidden = rows.children.length > 0;
        // Tell the page something changed (like typing in the text box would).
        textarea.dispatchEvent(new Event("input", { bubbles: true }));
    };

    const addRow = (label = "", url = "") => {
        const row = template.content.firstElementChild.cloneNode(true);
        row.querySelector("[data-menu-label]").value = label;
        row.querySelector("[data-menu-url]").value = url;
        rows.appendChild(row);
        return row;
    };

    for (const line of textarea.value.split("\n")) {
        if (!line.trim()) continue;
        const [label, ...url] = line.split("|");
        addRow(label.trim(), url.join("|").trim());
    }

    textarea.hidden = true;
    editor.hidden = false;
    empty.hidden = rows.children.length > 0;

    document.getElementById("menu-add").addEventListener("click", () => {
        addRow().querySelector("[data-menu-label]").focus();
        save();
    });

    rows.addEventListener("input", (event) => {
        // Picking one of your pages fills in its name if there's none yet.
        if (event.target.matches("[data-menu-url]")) {
            const choice = suggestions.find((option) => option.value === event.target.value);
            const label = event.target.closest(".menu-row").querySelector("[data-menu-label]");
            if (choice && !label.value) label.value = choice.label;
        }
        save();
    });

    rows.addEventListener("click", (event) => {
        const button = event.target.closest("button");
        if (!button) return;
        const row = button.closest(".menu-row");
        if (button.hasAttribute("data-menu-remove")) {
            row.remove();
        } else if (button.dataset.menuMove === "-1" && row.previousElementSibling) {
            row.previousElementSibling.before(row);
            button.focus();
        } else if (button.dataset.menuMove === "1" && row.nextElementSibling) {
            row.nextElementSibling.after(row);
            button.focus();
        }
        save();
    });
})();

// Email: choosing a provider fills in its mail server, port and security, and
// shows what kind of password it needs. "Other" shows the fields to fill in.
(() => {
    const provider = document.getElementById("smtp_provider");
    if (!provider) return;
    const host = document.getElementById("smtp_host");
    const port = document.getElementById("smtp_port");
    const security = document.getElementById("smtp_security");
    const username = document.getElementById("smtp_username");
    const from = document.getElementById("smtp_from");
    const serverFields = document.querySelectorAll("[data-server-field]");
    const hints = document.querySelectorAll("[data-provider-hint]");

    const show = (option) => {
        const known = Boolean(option.dataset.host);
        // The server details only need typing for "Other".
        serverFields.forEach((field) => (field.hidden = option.value !== "other"));
        hints.forEach((hint) => (hint.hidden = hint.dataset.providerHint !== option.dataset.hint));
        return known;
    };

    provider.addEventListener("change", () => {
        const option = provider.selectedOptions[0];
        if (show(option)) {
            host.value = option.dataset.host;
            port.value = option.dataset.port;
            security.value = option.dataset.security;
            if (!username.value && from.value) username.value = from.value;
        } else if (option.value === "other") {
            host.focus();
        }
    });

    // Start with the provider the saved mail server belongs to.
    const saved = [...provider.options].find((o) => o.dataset.host && o.dataset.host === host.value.trim());
    provider.value = saved ? saved.value : host.value.trim() ? "other" : "";
    show(provider.selectedOptions[0]);
    document.getElementById("provider-group").hidden = false;
})();

// Time zone: one click picks the one this computer uses.
(() => {
    const button = document.getElementById("timezone-detect");
    const select = document.getElementById("timezone");
    if (!button || !select) return;
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (!zone || ![...select.options].some((o) => o.value === zone) || select.value === zone) return;
    button.hidden = false;
    button.addEventListener("click", () => {
        select.value = zone;
        select.dispatchEvent(new Event("change", { bubbles: true }));
        button.hidden = true;
    });
})();

// Updates: installing automatically only works with the daily check on.
(() => {
    const daily = document.querySelector("[data-check-daily]");
    const auto = document.querySelector("[data-install-auto]");
    if (!daily || !auto) return;
    const sync = () => {
        auto.disabled = !daily.checked;
        if (!daily.checked) auto.checked = false;
    };
    daily.addEventListener("change", sync);
    sync();
})();
