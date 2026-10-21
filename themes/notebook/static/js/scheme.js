// Light or dark, chosen by a visitor with the button in the header. Each
// click goes device → light → dark → device, like the switch in the admin.
// The choice stays in this visitor's own browser (no cookie, nothing sent to
// the server); until they choose, and after "device" again, the site follows
// their device. A saved choice is applied before the page is drawn by
// `scheme_script` in the page (see colors.html); this file only runs the
// button, so it loads with `defer` and never holds the page back.
(() => {
    const key = "bloogla:scheme";
    const root = document.documentElement;

    // The button says which choice is on ("Light or dark: always dark"); the
    // icon follows <html data-scheme> in style.css.
    const label = (button) => {
        const state = root.dataset.scheme || "device";
        button.setAttribute("aria-label", button.dataset[state]);
        button.title = button.dataset[state];
    };

    document.addEventListener("click", (event) => {
        const button = event.target.closest("[data-scheme-toggle]");
        if (!button) return;
        const next = { device: "light", light: "dark", dark: "" }[root.dataset.scheme || "device"];
        // Colors glide for a moment instead of flipping (see style.css).
        root.classList.add("scheme-changing");
        setTimeout(() => root.classList.remove("scheme-changing"), 350);
        try {
            if (next) localStorage.setItem(key, next);
            else localStorage.removeItem(key);
        } catch {
            // Not remembered, but this page still changes.
        }
        if (next) root.dataset.scheme = next;
        else delete root.dataset.scheme;
        label(button);
    });

    document.addEventListener("DOMContentLoaded", () =>
        document.querySelectorAll("[data-scheme-toggle]").forEach(label));
})();
