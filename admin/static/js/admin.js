// Behaviour shared by every admin page. Loaded by admin/templates/layout.html
// (and the setup page). The admin panel's Content Security Policy forbids
// inline scripts, so page behaviour lives in files like this one and is
// switched on with data- attributes in the HTML:
//
//   data-delete-url="/admin/posts/1"   button that asks before deleting
//   data-open-modal="theme-modal"      button that opens a modal by id
//   data-modal-close                   button that closes its modal
//   data-copy="text"                   button that copies text (data-copied /
//                                      data-copy-failed: what it says after)
//   data-reset-on-success              htmx form emptied after a successful save
//   data-clear-on-success="#id"        element emptied after a successful save
//   data-navigate="lang"               <select> that reloads with ?lang=<value>
//   data-empty-text="..."              list that shows this text when empty
//   data-menu-toggle                   button that opens the menu on phones

// ---- Modals ----

function openModal(modal) {
    modal.classList.add("active");
    const first = modal.querySelector("[data-modal-close], button, input");
    if (first) first.focus();
}

function closeModal(modal) {
    modal.classList.remove("active");
}

document.addEventListener("click", (event) => {
    const opener = event.target.closest("[data-open-modal]");
    if (opener) openModal(document.getElementById(opener.dataset.openModal));

    const closer = event.target.closest("[data-modal-close]");
    if (closer) closeModal(closer.closest(".modal-overlay"));
    if (event.target.classList.contains("modal-overlay")) closeModal(event.target);
});

document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    document.querySelectorAll(".modal-overlay.active").forEach(closeModal);
});

// ---- Media picker: openMediaPicker((url, alt) => ...) ----

let mediaPickerCallback = null;

function openMediaPicker(callback) {
    mediaPickerCallback = callback;
    const modal = document.getElementById("media-modal");
    htmx.ajax("GET", "/admin/media/picker", { target: "#picker-body" });
    modal.classList.add("active");
}

document.getElementById("media-modal")?.addEventListener("click", (event) => {
    const item = event.target.closest(".picker-item");
    if (!item || !mediaPickerCallback) return;
    mediaPickerCallback(item.dataset.url, item.dataset.alt);
    event.currentTarget.classList.remove("active");
});

// ---- Delete buttons, confirmed in a modal ----
// <button data-delete-url="/admin/posts/1" data-delete-target="#post-1"
//         data-confirm-title="..." data-confirm-text="..." data-delete-redirect="/admin/posts">

const confirmModal = document.getElementById("confirm-modal");
const confirmButton = document.getElementById("confirm-delete");
let deleteTrigger = null;

document.addEventListener("click", (event) => {
    const trigger = event.target.closest("[data-delete-url]");
    if (!trigger || !confirmModal) return;
    deleteTrigger = trigger;

    document.getElementById("confirm-title").textContent =
        trigger.dataset.confirmTitle || confirmModal.dataset.defaultTitle;
    document.getElementById("confirm-text").textContent =
        trigger.dataset.confirmText || confirmModal.dataset.defaultText;
    document.getElementById("confirm-error").hidden = true;

    confirmButton.setAttribute("hx-delete", trigger.dataset.deleteUrl);
    if (trigger.dataset.deleteTarget) {
        confirmButton.setAttribute("hx-target", trigger.dataset.deleteTarget);
        confirmButton.setAttribute("hx-swap", "delete");
    } else {
        confirmButton.removeAttribute("hx-target");
        confirmButton.setAttribute("hx-swap", "none");
    }
    htmx.process(confirmButton);
    openModal(confirmModal);
});

document.addEventListener("htmx:afterRequest", (event) => {
    if (!confirmButton || event.detail.elt !== confirmButton) return;
    if (event.detail.successful) {
        const redirect = deleteTrigger && deleteTrigger.dataset.deleteRedirect;
        if (redirect) {
            window.skipUnloadWarning = true;
            window.location.href = redirect;
            return;
        }
        closeModal(confirmModal);
    } else {
        document.getElementById("confirm-error").hidden = false;
    }
});

// ---- htmx forms that empty themselves after saving ----
// Replies with an error message (class alert-error) keep what was typed.

document.addEventListener("htmx:afterRequest", (event) => {
    const form = event.detail.elt;
    if (!(form instanceof HTMLFormElement) || !form.hasAttribute("data-reset-on-success")) return;
    const reply = event.detail.xhr ? event.detail.xhr.responseText : "";
    if (!event.detail.successful || reply.includes("alert-error")) return;
    form.reset();
    const clear = form.dataset.clearOnSuccess;
    if (clear) document.querySelector(clear).innerHTML = "";
});

// ---- Copy buttons ----

document.addEventListener("click", async (event) => {
    const button = event.target.closest("[data-copy]");
    if (!button) return;
    const label = button.textContent;
    try {
        await navigator.clipboard.writeText(button.dataset.copy);
        button.textContent = button.dataset.copied || label;
    } catch {
        button.textContent = button.dataset.copyFailed || label;
    }
    setTimeout(() => (button.textContent = label), 1200);
});

// ---- Selects that reload the page with a new query value ----

document.addEventListener("change", (event) => {
    const select = event.target.closest("select[data-navigate]");
    if (!select) return;
    const params = new URLSearchParams(window.location.search);
    params.set(select.dataset.navigate, select.value);
    window.location.search = params.toString();
});

// ---- Navigation: highlight the current section and its sub-pages ----

document.querySelectorAll(".nav-list .nav-item").forEach((link) => {
    const href = link.getAttribute("href");
    const path = window.location.pathname;
    const active = href === "/admin" ? path === "/admin" : path === href || path.startsWith(href + "/");
    if (active) {
        link.classList.add("active");
        link.setAttribute("aria-current", "page");
    }
});

// ---- Lists marked with data-empty-text show a message when their last item goes away ----
// The message is the list's `[data-empty-for]` element if the page has one,
// otherwise a paragraph with the data-empty-text.

function syncEmptyList(list) {
    const hasItems = list.querySelector("[data-item]") !== null;
    let message = document.querySelector(`[data-empty-for="${list.id}"]`);
    list.hidden = !hasItems;
    if (!message && !hasItems) {
        message = document.createElement("p");
        message.className = "empty-state";
        message.dataset.emptyFor = list.id;
        message.textContent = list.dataset.emptyText;
        list.after(message);
    }
    if (message) message.hidden = hasItems;
}

// Each list watches its own items, so it updates however an item was added
// or removed (deleted, approved, created...).
document.querySelectorAll("[data-empty-text]").forEach((list) => {
    new MutationObserver(() => syncEmptyList(list)).observe(list, { childList: true, subtree: true });
});

// ---- The menu on phones: opens and closes with the Menu button ----

const menuToggle = document.querySelector("[data-menu-toggle]");
if (menuToggle) {
    const sidebar = menuToggle.closest(".sidebar");
    const setMenu = (open) => {
        sidebar.classList.toggle("menu-open", open);
        menuToggle.setAttribute("aria-expanded", String(open));
    };
    menuToggle.addEventListener("click", () => setMenu(!sidebar.classList.contains("menu-open")));
    document.addEventListener("keydown", (event) => {
        if (event.key === "Escape" && sidebar.classList.contains("menu-open")) {
            setMenu(false);
            menuToggle.focus();
        }
    });
}
