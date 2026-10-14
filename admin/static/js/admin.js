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
//   data-tabs / data-tab-panel="x"     tabs that show one panel at a time
//   data-live (with an id)             part of the page fetched again after a
//                                      change, so it never shows old values
//
// And, without any attribute: every password field gets an eye button to
// show what was typed, a refused form shakes its button, a saved one
// shows a check that draws itself, removed rows close up smoothly and counts
// in the menu bounce when they change (see "Feel" below and "MOTION" in
// style.css).

// ---- A message kept across a reload ----
// Some changes affect the whole page, like its language. The server then
// answers with HX-Trigger: reload-page and the message to show; the page
// reloads (smoothly, see "MOTION" in style.css) and the message
// appears again in the same form (found by its hx-post address; Settings has
// two forms with the same one, so also by which of them it was).

const FLASH_KEY = "bloogla:flash";
const formsPostingTo = (url) => [...document.querySelectorAll("form[hx-post]")].filter((f) => f.getAttribute("hx-post") === url);

function reloadWithMessage(form, html) {
    const url = form ? form.getAttribute("hx-post") : "";
    const flash = {
        path: location.pathname,
        url,
        index: formsPostingTo(url).indexOf(form),
        html,
        hash: location.hash,
    };
    try {
        sessionStorage.setItem(FLASH_KEY, JSON.stringify(flash));
    } catch { }
    // Without the #tab this is a real page load, which browsers can cross-fade;
    // the tab is opened again from the flash below.
    location.replace(location.pathname + location.search);
}

let flash = null;
try {
    flash = JSON.parse(sessionStorage.getItem(FLASH_KEY));
    sessionStorage.removeItem(FLASH_KEY);
} catch { }
if (flash && flash.path === location.pathname) {
    if (flash.hash && !location.hash) history.replaceState(null, "", flash.hash);
    const status = formsPostingTo(flash.url)[flash.index]?.querySelector(".form-status");
    if (status) status.innerHTML = flash.html;
}

// ---- After installing an update ----
// Bloogla stops and starts again as the new version. Wait until it answers
// again (it's down for a few seconds), then reload the page.

function reloadAfterRestart() {
    const started = Date.now();
    const poll = async () => {
        try {
            const response = await fetch("/admin", { cache: "no-store" });
            if (response.ok && Date.now() - started > 4000) {
                location.reload();
                return;
            }
        } catch {
            // Still restarting.
        }
        setTimeout(poll, 1500);
    };
    setTimeout(poll, 2500);
}

// ---- Live parts: refreshed after every change, so nothing needs a reload ----
// Elements with data-live and an id (the site name in the sidebar, the counts
// above lists...) are replaced by their new version from the server.

function replaceLiveParts(doc) {
    document.title = doc.title;
    document.querySelectorAll("[data-live][id]").forEach((part) => {
        const fresh = doc.getElementById(part.id);
        if (!fresh || fresh.outerHTML === part.outerHTML) return;
        part.replaceWith(fresh);
        htmx.process(fresh);
        // A count next to a menu item bounces once when it changes.
        if (fresh.classList.contains("nav-badge") && !fresh.hidden && fresh.textContent !== part.textContent) {
            fresh.classList.add("pop");
        }
    });
}

async function refreshLiveParts() {
    if (!document.querySelector("[data-live][id]")) return;
    try {
        const response = await fetch(location.href);
        if (!response.ok) return;
        replaceLiveParts(new DOMParser().parseFromString(await response.text(), "text/html"));
    } catch {
        // Offline for a moment: the parts keep their old text until the next change.
    }
}

document.addEventListener("htmx:afterRequest", (event) => {
    const { elt, xhr, successful, requestConfig } = event.detail;
    if (!successful || !xhr || requestConfig.verb === "get") return;
    if (xhr.getResponseHeader("HX-Trigger") === "reload-page") {
        reloadWithMessage(elt.closest("form"), xhr.responseText);
        return;
    }
    if (xhr.getResponseHeader("HX-Trigger") === "restarting") {
        reloadAfterRestart();
        return;
    }
    // Errors change nothing; the media chooser only adds images.
    if (xhr.responseText.includes("alert-error") || elt.closest("#media-modal")) return;
    refreshLiveParts();
});

// Validation and account slowdowns can return HTML messages: show them.
document.addEventListener("htmx:beforeSwap", (event) => {
    const xhr = event.detail.xhr;
    const htmlSlowdown = xhr.status === 429
        && (xhr.getResponseHeader("Content-Type") || "").startsWith("text/html");
    if (xhr.status === 422 || htmlSlowdown) {
        event.detail.shouldSwap = true;
        event.detail.isError = false;
    }
});

// ---- Password fields: an eye to show or hide what was typed ----
// Added to every password field, also ones htmx brings in later. The labels
// come from <body data-show-password data-hide-password> (translated).
// Browsers' own eyes are hidden in style.css (Edge shows one only sometimes).

const EYE = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/></svg>';
const EYE_OFF = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 3l18 18"/><path d="M10.6 5.1A10.9 10.9 0 0 1 12 5c6.4 0 10 7 10 7a17.6 17.6 0 0 1-3.2 4.1"/><path d="M6.6 6.6C3.9 8.4 2 12 2 12s3.6 7 10 7a10.6 10.6 0 0 0 5.4-1.4"/><path d="M9.9 9.9a3 3 0 0 0 4.2 4.2"/></svg>';

function addPasswordEyes(root) {
    const labels = document.body.dataset;
    root.querySelectorAll?.('input[type="password"]:not([data-has-eye])').forEach((input) => {
        input.dataset.hasEye = "";
        const field = document.createElement("span");
        field.className = "password-field";
        input.before(field);
        field.append(input);

        const button = document.createElement("button");
        button.type = "button";
        button.className = "password-eye";
        const show = (visible) => {
            input.type = visible ? "text" : "password";
            button.innerHTML = visible ? EYE_OFF : EYE;
            button.setAttribute("aria-label", visible ? labels.hidePassword : labels.showPassword);
            button.setAttribute("aria-pressed", String(visible));
        };
        show(false);
        button.addEventListener("click", () => {
            show(input.type === "password");
            // Keep typing where you were.
            input.focus();
            const end = input.value.length;
            input.setSelectionRange(end, end);
        });
        field.append(button);
        // Never send a password as ordinary text: browsers would remember it
        // like any typed word.
        input.form?.addEventListener("submit", () => show(false));
    });
}

addPasswordEyes(document);
document.addEventListener("htmx:load", (event) => addPasswordEyes(event.target));

// ---- Feel: "no" shakes, "done" draws a check ----

// Shake an element left and right a few times (see .shake in style.css).
function shake(element) {
    element.classList.remove("shake");
    void element.offsetWidth; // Start again even if it's already shaking.
    element.classList.add("shake");
    element.addEventListener("animationend", () => element.classList.remove("shake"), { once: true });
}

// A check mark drawn as a line, so CSS can draw it stroke by stroke.
function drawnCheck(className) {
    const svgNs = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(svgNs, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("class", className);
    svg.setAttribute("aria-hidden", "true");
    const path = document.createElementNS(svgNs, "path");
    path.setAttribute("d", "M5 12.5l4.5 4.5L19 7.5");
    svg.append(path);
    return svg;
}

// After saving: the button shows a check for a moment, then its label again.
function showDone(button) {
    if (button.classList.contains("is-done")) return;
    // The label goes transparent; the check keeps the label's color.
    button.style.setProperty("--check-color", getComputedStyle(button).color);
    const check = drawnCheck("btn-check");
    button.append(check);
    button.classList.add("is-done");
    setTimeout(() => {
        button.classList.remove("is-done");
        check.remove();
    }, 1300);
}

// The button that sent an htmx request, if it's still on the page.
function sendingButton(elt, target) {
    for (const root of [elt, target]) {
        if (!root || !root.isConnected) continue;
        if (root.matches("button, .btn")) return root;
        const button = (root.closest("form") || root).querySelector?.('[type="submit"]');
        if (button) return button;
    }
    return null;
}

document.addEventListener("htmx:afterSettle", (event) => {
    const { elt, target, xhr, requestConfig } = event.detail;
    if (!xhr || !requestConfig || requestConfig.verb === "get") return;
    const button = sendingButton(elt, target);
    // The delete confirmation closes by itself; reloads and restarts move on.
    if (!button || button.closest(".modal-overlay")) return;
    if (xhr.status === 422) {
        shake(button);
        return;
    }
    const trigger = xhr.getResponseHeader("HX-Trigger");
    const done = xhr.status >= 200 && xhr.status < 300 && !xhr.responseText.includes("alert-error");
    if (done && trigger !== "reload-page" && trigger !== "restarting") showDone(button);
});

// ---- Feedback while waiting ----
// htmx marks a form or button with .htmx-request while it saves; ordinary
// forms that load a new page get .is-busy on their button (see style.css).

document.addEventListener("submit", (event) => {
    if (event.defaultPrevented || !event.submitter) return;
    event.submitter.classList.add("is-busy");
});
// Coming back with the Back button shows the page as it was left.
window.addEventListener("pageshow", () => {
    document.querySelectorAll(".is-busy").forEach((button) => button.classList.remove("is-busy"));
});

// "Saved." fades away after a while; errors stay until the next try.
document.addEventListener("htmx:afterSettle", (event) => {
    event.detail.target.querySelectorAll?.(".form-status > .alert-success").forEach((alert) => {
        setTimeout(() => {
            alert.classList.add("fading");
            setTimeout(() => alert.remove(), 300);
        }, 5000);
    });
});

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
        // The item fades out (see .htmx-swapping in style.css) before it goes.
        confirmButton.setAttribute("hx-target", trigger.dataset.deleteTarget);
        confirmButton.setAttribute("hx-swap", "delete swap:280ms");
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
        const removed = deleteTrigger && deleteTrigger.dataset.deleteTarget
            && document.querySelector(deleteTrigger.dataset.deleteTarget);
        if (removed) closeUp(removed);
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

// A row being removed shrinks to nothing, so the rows below glide up into its
// place (htmx removes it once the time in hx-swap="delete swap:…" is over).
// Cards in a grid just fade: their neighbours move to new spots instead.
function closeUp(element) {
    if (getComputedStyle(element.parentElement).display.includes("grid")) return;
    element.style.height = `${element.offsetHeight}px`;
    void element.offsetHeight; // Take the height as the starting point.
    element.classList.add("collapsing");
}

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
        button.prepend(drawnCheck("copy-check"));
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

// Lists update however an item was added or removed (deleted, approved,
// created, or the whole list replaced after an upload).
new MutationObserver(() => {
    document.querySelectorAll("[data-empty-text][id]").forEach(syncEmptyList);
}).observe(document.body, { childList: true, subtree: true });

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

// ---- Tabs: one panel at a time (Settings, Profile) ----
// <nav data-tabs><a href="#email" data-tab="email">...</a></nav> and
// <div data-tab-panel="email">. The open tab is in the address (#email), so
// reloading or sharing the link opens it again. Without JavaScript every
// panel shows, one under the other.

const tabs = document.querySelector("[data-tabs]");
if (tabs) {
    const links = [...tabs.querySelectorAll("[data-tab]")];
    const panels = [...document.querySelectorAll("[data-tab-panel]")];
    const openTab = (name) => {
        const known = links.some((link) => link.dataset.tab === name);
        const current = known ? name : links[0].dataset.tab;
        links.forEach((link) => {
            const active = link.dataset.tab === current;
            link.classList.toggle("active", active);
            link.setAttribute("aria-selected", String(active));
        });
        panels.forEach((panel) => (panel.hidden = panel.dataset.tabPanel !== current));
    };
    tabs.addEventListener("click", (event) => {
        const link = event.target.closest("[data-tab]");
        if (!link) return;
        event.preventDefault();
        history.replaceState(null, "", "#" + link.dataset.tab);
        openTab(link.dataset.tab);
    });
    tabs.hidden = false;
    document.body.classList.add("has-tabs");
    openTab(location.hash.slice(1));
}
