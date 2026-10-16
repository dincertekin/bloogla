// Light or dark, chosen by a visitor with the sun/moon button in the header.
// Loaded in <head>, before the page is drawn, so a saved choice shows from
// the first moment instead of flashing the other colors. The choice stays in
// this visitor's own browser (no cookie, nothing sent to the server); until
// they choose, the site follows their device.
(() => {
    const key = "bloogla:scheme";
    const root = document.documentElement;
    try {
        const saved = localStorage.getItem(key);
        if (saved === "light" || saved === "dark") root.dataset.scheme = saved;
    } catch {
        // Storage turned off: follow the device.
    }

    const isDark = () =>
        root.dataset.scheme
            ? root.dataset.scheme === "dark"
            : window.matchMedia("(prefers-color-scheme: dark)").matches;

    // The button says what clicking it does ("Switch to dark mode").
    const label = (button) => {
        button.setAttribute("aria-label", isDark() ? button.dataset.toLight : button.dataset.toDark);
        button.setAttribute("aria-pressed", String(isDark()));
    };

    document.addEventListener("click", (event) => {
        const button = event.target.closest("[data-scheme-toggle]");
        if (!button) return;
        const next = isDark() ? "light" : "dark";
        // Colors glide for a moment instead of flipping (see style.css).
        root.classList.add("scheme-changing");
        setTimeout(() => root.classList.remove("scheme-changing"), 350);
        root.dataset.scheme = next;
        try {
            localStorage.setItem(key, next);
        } catch {
            // Not remembered, but this page still changes.
        }
        label(button);
    });

    document.addEventListener("DOMContentLoaded", () =>
        document.querySelectorAll("[data-scheme-toggle]").forEach(label));
})();
